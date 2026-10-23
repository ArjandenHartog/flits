//! GPU compositor (wgpu: D3D12 / Vulkan / Metal). Each draw op becomes one quad;
//! the finished frame is converted to YUV 4:2:0 on the GPU and read back for the encoder.

use super::{Op, Painter, Src};
use crate::scene::color::Rgba;
use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_skia::{Pixmap, Transform};
use wgpu::util::DeviceExt;

const COMPOSITE_WGSL: &str = include_str!("composite.wgsl");
const YUV_WGSL: &str = include_str!("yuv.wgsl");
/// keep at most this many bytes of static textures resident
const TEX_BUDGET: u64 = 1_200 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    m: [f32; 4],
    t: [f32; 4],
    shape: [f32; 4],
    uvm: [f32; 4],
    uvt: [f32; 4],
    color: [f32; 4],
    cm: [f32; 4],
    ct: [f32; 4],
    cr: [f32; 4],
}

enum TexRef<'a> {
    White,
    Static(&'a Pixmap),
    Video { key: usize, ver: i64, data: &'a [u8], w: u32, h: u32 },
}

/// Collects draw ops for one frame.
#[derive(Default)]
pub struct GpuPainter<'a> {
    ops: Vec<(Inst, TexRef<'a>)>,
}

fn det_scale(t: &Transform) -> f32 {
    (t.sx * t.sy - t.kx * t.ky).abs().sqrt().max(1e-6)
}

impl<'a> Painter<'a> for GpuPainter<'a> {
    fn draw(&mut self, op: Op<'a>) {
        let [x0, y0, x1, y1] = op.shape;
        if x1 <= x0 || y1 <= y0 || op.opacity <= 0.0 {
            return;
        }
        let ts = op.ts;
        let aa = det_scale(&ts);
        let mut inst = Inst {
            m: [ts.sx, ts.kx, ts.ky, ts.sy],
            t: [ts.tx, ts.ty, op.opacity, op.radius],
            shape: op.shape,
            uvm: [0.0; 4],
            uvt: [0.0, 0.0, 1.0, aa],
            color: [0.0; 4],
            cm: [0.0; 4],
            ct: [0.0; 4],
            cr: [0.0; 4],
        };
        if let Some(c) = op.clip {
            if let Some(inv) = c.ts.invert() {
                inst.cm = [inv.sx, inv.kx, inv.ky, inv.sy];
                inst.ct = [inv.tx, inv.ty, 1.0, det_scale(&c.ts)];
                inst.cr = c.rect;
            }
        }
        let uv_for = |w: u32, h: u32, inst: &mut Inst| {
            if let Some(inv) = op.uv.invert() {
                let n = inv.post_scale(1.0 / w as f32, 1.0 / h as f32);
                inst.uvm = [n.sx, n.kx, n.ky, n.sy];
                inst.uvt = [n.tx, n.ty, 0.0, aa];
            }
        };
        let tex = match op.src {
            Src::Solid(c) => {
                inst.color = [c.r * c.a, c.g * c.a, c.b * c.a, c.a];
                TexRef::White
            }
            Src::Image(pm) => {
                uv_for(pm.width(), pm.height(), &mut inst);
                TexRef::Static(pm)
            }
            Src::Frame { key, ver, data, w, h } => {
                uv_for(w, h, &mut inst);
                TexRef::Video { key, ver, data, w, h }
            }
        };
        self.ops.push((inst, tex));
    }
}

struct Cached {
    bg: Arc<wgpu::BindGroup>,
    bytes: u64,
    last: Instant,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    yuv: wgpu::ComputePipeline,
    tex_layout: wgpu::BindGroupLayout,
    uni_layout: wgpu::BindGroupLayout,
    yuv_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: Arc<wgpu::BindGroup>,
    statics: Mutex<(HashMap<usize, Cached>, u64)>,
    pub name: String,
}

/// What to read back after compositing.
#[derive(Clone, Copy, PartialEq)]
pub enum Readback {
    /// planar YUV 4:2:0 (BT.709 limited), `padded_width(w) x h`
    Yuv420,
    /// premultiplied RGBA, `w x h`
    Rgba,
}

/// The YUV path works on 8-pixel groups; frames are padded to this width and cropped by the encoder.
pub fn padded_width(w: u32) -> u32 {
    w.div_ceil(8) * 8
}

/// Per-worker GPU resources.
pub struct GpuTarget {
    w: u32,
    h: u32,
    /// real (unpadded) width
    real_w: u32,
    tex: wgpu::Texture,
    rgba_buf: Option<wgpu::Buffer>,
    view: wgpu::TextureView,
    uni_bg: wgpu::BindGroup,
    inst: wgpu::Buffer,
    inst_cap: usize,
    yuv_buf: wgpu::Buffer,
    read_buf: wgpu::Buffer,
    yuv_bg: wgpu::BindGroup,
    videos: HashMap<usize, (i64, wgpu::Texture, Arc<wgpu::BindGroup>, u32, u32)>,
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..Default::default() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| anyhow!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(anyhow!("only a software GPU adapter is available"));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("flits"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("creating GPU device")?;

        let (render_src, yuv_src) = (COMPOSITE_WGSL, YUV_WGSL);
        let render_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render"),
            source: wgpu::ShaderSource::Wgsl(render_src.into()),
        });
        let yuv_mod = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("yuv"), source: wgpu::ShaderSource::Wgsl(yuv_src.into()) });

        let uni_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let yuv_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&uni_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("composite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &render_mod,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Inst>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_mod,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let yuv_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&yuv_layout],
            push_constant_ranges: &[],
        });
        let yuv = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("yuv"),
            layout: Some(&yuv_pl),
            module: &yuv_mod,
            entry_point: Some("yuv"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let white = Arc::new(make_tex_bg(&device, &queue, &tex_layout, &sampler, 1, 1, &[255, 255, 255, 255]).0);
        Ok(Gpu {
            device,
            queue,
            pipeline,
            yuv,
            tex_layout,
            uni_layout,
            yuv_layout,
            sampler,
            white,
            statics: Mutex::new((HashMap::new(), 0)),
            name: format!("{} ({:?})", info.name, info.backend),
        })
    }

    fn texture_bg(&self, w: u32, h: u32, data: &[u8]) -> (wgpu::BindGroup, wgpu::Texture) {
        make_tex_bg(&self.device, &self.queue, &self.tex_layout, &self.sampler, w, h, data)
    }

    fn static_bg(&self, pm: &Pixmap) -> Arc<wgpu::BindGroup> {
        let key = pm as *const Pixmap as usize;
        let mut guard = self.statics.lock().unwrap();
        let (map, bytes) = &mut *guard;
        if let Some(c) = map.get_mut(&key) {
            c.last = Instant::now();
            return c.bg.clone();
        }
        let size = pm.data().len() as u64;
        if *bytes + size > TEX_BUDGET {
            // evict textures nobody used in the last few seconds (workers move forward in time)
            let now = Instant::now();
            map.retain(|_, c| now.duration_since(c.last).as_secs_f32() < 3.0);
            *bytes = map.values().map(|c| c.bytes).sum();
        }
        let (bg, _tex) = self.texture_bg(pm.width(), pm.height(), pm.data());
        let bg = Arc::new(bg);
        map.insert(key, Cached { bg: bg.clone(), bytes: size, last: Instant::now() });
        *bytes += size;
        bg
    }

    pub fn target(&self, real_w: u32, h: u32) -> GpuTarget {
        let w = padded_width(real_w);
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let uni = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[w as f32, h as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uni_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.uni_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uni.as_entire_binding() }],
        });
        let yuv_size = (w * h * 3 / 2) as u64;
        let yuv_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: yuv_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: yuv_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dims = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&[w, h, 0u32, 0u32]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let yuv_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.yuv_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: yuv_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: dims.as_entire_binding() },
            ],
        });
        let inst_cap = 256;
        let inst = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (inst_cap * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        GpuTarget { w, h, real_w, tex, rgba_buf: None, view, uni_bg, inst, inst_cap, yuv_buf, read_buf, yuv_bg, videos: HashMap::new() }
    }

    /// Composite the ops and read the frame back into `out` (YUV for encoding, RGBA for stills).
    pub fn render(&self, tg: &mut GpuTarget, bg: Rgba, painter: GpuPainter, mode: Readback, out: &mut Vec<u8>) -> Result<()> {
        let ops = painter.ops;
        // resolve textures
        let mut bgs: Vec<Arc<wgpu::BindGroup>> = Vec::with_capacity(ops.len());
        for (_, t) in &ops {
            bgs.push(match t {
                TexRef::White => self.white.clone(),
                TexRef::Static(pm) => self.static_bg(pm),
                TexRef::Video { key, ver, data, w, h } => {
                    let fresh = match tg.videos.get(key) {
                        Some(v) => v.3 != *w || v.4 != *h,
                        None => true,
                    };
                    if fresh {
                        let (b, tex) = self.texture_bg(*w, *h, data);
                        tg.videos.insert(*key, (*ver, tex, Arc::new(b), *w, *h));
                    }
                    let v = tg.videos.get_mut(key).unwrap();
                    if v.0 != *ver {
                        self.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &v.1,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            data,
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
                            wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
                        );
                        v.0 = *ver;
                    }
                    v.2.clone()
                }
            });
        }
        // drop decoders' textures that weren't used this frame
        let used: Vec<usize> = ops.iter().filter_map(|(_, t)| if let TexRef::Video { key, .. } = t { Some(*key) } else { None }).collect();
        tg.videos.retain(|k, _| used.contains(k));

        if ops.len() > tg.inst_cap {
            tg.inst_cap = ops.len().next_power_of_two();
            tg.inst = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (tg.inst_cap * std::mem::size_of::<Inst>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let insts: Vec<Inst> = ops.iter().map(|(i, _)| *i).collect();
        if !insts.is_empty() {
            self.queue.write_buffer(&tg.inst, 0, bytemuck::cast_slice(&insts));
        }

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tg.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: (bg.r * bg.a) as f64,
                            g: (bg.g * bg.a) as f64,
                            b: (bg.b * bg.a) as f64,
                            a: bg.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &tg.uni_bg, &[]);
            pass.set_vertex_buffer(0, tg.inst.slice(..));
            for (i, b) in bgs.iter().enumerate() {
                pass.set_bind_group(1, b.as_ref(), &[]);
                pass.draw(0..6, i as u32..i as u32 + 1);
            }
        }
        let row = (tg.real_w * 4).div_ceil(256) * 256;
        let read: &wgpu::Buffer = match mode {
            Readback::Yuv420 => {
                let mut pass = enc.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.yuv);
                pass.set_bind_group(0, &tg.yuv_bg, &[]);
                pass.dispatch_workgroups((tg.w / 8).div_ceil(16), (tg.h / 2).div_ceil(8), 1);
                drop(pass);
                enc.copy_buffer_to_buffer(&tg.yuv_buf, 0, &tg.read_buf, 0, (tg.w * tg.h * 3 / 2) as u64);
                &tg.read_buf
            }
            Readback::Rgba => {
                if tg.rgba_buf.is_none() {
                    tg.rgba_buf = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: (row * tg.h) as u64,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }));
                }
                let buf = tg.rgba_buf.as_ref().unwrap();
                enc.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tg.tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: buf,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(tg.h) },
                    },
                    wgpu::Extent3d { width: tg.real_w, height: tg.h, depth_or_array_layers: 1 },
                );
                buf
            }
        };
        let idx = self.queue.submit([enc.finish()]);

        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        loop {
            self.device.poll(wgpu::PollType::WaitForSubmissionIndex(idx.clone())).map_err(|e| anyhow!("GPU poll failed: {e}"))?;
            match rx.try_recv() {
                Ok(r) => {
                    r.map_err(|e| anyhow!("GPU readback failed: {e}"))?;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => continue,
                Err(e) => return Err(anyhow!("GPU readback: {e}")),
            }
        }
        {
            let data = slice.get_mapped_range();
            out.clear();
            match mode {
                Readback::Yuv420 => out.extend_from_slice(&data),
                Readback::Rgba => {
                    for y in 0..tg.h as usize {
                        let s = y * row as usize;
                        out.extend_from_slice(&data[s..s + tg.real_w as usize * 4]);
                    }
                }
            }
        }
        read.unmap();
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn make_tex_bg(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    data: &[u8],
) -> (wgpu::BindGroup, wgpu::Texture) {
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        data,
    );
    let view = tex.create_view(&Default::default());
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (bg, tex)
}
