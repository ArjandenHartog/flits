//! GPU compositor (wgpu: Vulkan / D3D12 / Metal).
//!
//! * every draw op is one instanced quad; rounded corners and anti-aliasing come
//!   from a signed-distance function in the fragment shader;
//! * video frames are uploaded as planar YUV (8- or 16-bit) and converted to RGB,
//!   including HDR -> SDR tone mapping, in the shader;
//! * the finished frame is converted to YUV 4:2:0 by a compute shader and read back
//!   for the encoder, so the CPU barely touches pixels.

use super::{Op, Painter, Src};
use crate::media::colorspace::{FrameFormat, NO_ADJUST};
use crate::scene::color::Rgba;
use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_skia::{Pixmap, Transform};
use wgpu::util::DeviceExt;

const COMPOSITE_WGSL: &str = include_str!("composite.wgsl");
const YUV_WGSL: &str = include_str!("yuv.wgsl");
/// keep at most this many bytes of static textures resident
const TEX_BUDGET: u64 = 1_200 * 1024 * 1024;

/// Per-quad data (vertex buffer, one entry per instance). Must match `VIn` in composite.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Inst {
    /// shape-local -> canvas: sx, kx, ky, sy
    m: [f32; 4],
    /// tx, ty, opacity, radius
    t: [f32; 4],
    /// x0, y0, x1, y1
    shape: [f32; 4],
    /// shape-local -> uv: sx, kx, ky, sy
    uvm: [f32; 4],
    /// tx, ty, mode (0 texture, 1 solid, 2 yuv video), px-per-unit
    uvt: [f32; 4],
    /// premultiplied solid colour
    color: [f32; 4],
    /// canvas -> clip-local
    cm: [f32; 4],
    /// tx, ty, has-clip, px-per-clip-unit
    ct: [f32; 4],
    /// clip rect
    cr: [f32; 4],
    /// video: transfer, matrix, full range, sample scale
    vid: [f32; 4],
    /// video colour grading: brightness, contrast, saturation, warmth
    adj: [f32; 4],
}

const MODE_TEXTURE: f32 = 0.0;
const MODE_SOLID: f32 = 1.0;
const MODE_YUV: f32 = 2.0;

enum TexRef<'a> {
    White,
    Static(&'a Pixmap),
    Video { key: usize, ver: i64, data: &'a [u8], w: u32, h: u32, fmt: FrameFormat },
}

/// Collects draw ops for one frame.
#[derive(Default)]
pub struct GpuPainter<'a> {
    ops: Vec<(Inst, TexRef<'a>)>,
}

impl GpuPainter<'_> {
    /// Identifies the frame these ops draw: equal fingerprints draw identical pixels (same quads,
    /// same textures, same video frame versions).
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.ops.len().hash(&mut h);
        for (inst, tex) in &self.ops {
            h.write(bytemuck::bytes_of(inst));
            match tex {
                TexRef::White => 0usize.hash(&mut h),
                TexRef::Static(pm) => (*pm as *const Pixmap as usize).hash(&mut h),
                TexRef::Video { key, ver, .. } => (key, ver).hash(&mut h),
            }
        }
        h.finish()
    }
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
            uvt: [0.0, 0.0, MODE_SOLID, aa],
            color: [0.0; 4],
            cm: [0.0; 4],
            ct: [0.0; 4],
            cr: [0.0; 4],
            vid: [0.0; 4],
            adj: NO_ADJUST,
        };
        if let Some(c) = op.clip {
            if let Some(inv) = c.ts.invert() {
                inst.cm = [inv.sx, inv.kx, inv.ky, inv.sy];
                // has-clip flag carries the clip's corner radius: 1 + radius
                inst.ct = [inv.tx, inv.ty, 1.0 + c.radius.max(0.0), det_scale(&c.ts)];
                inst.cr = c.rect;
            }
        }
        let uv_for = |w: u32, h: u32, mode: f32, inst: &mut Inst| {
            if let Some(inv) = op.uv.invert() {
                let n = inv.post_scale(1.0 / w as f32, 1.0 / h as f32);
                inst.uvm = [n.sx, n.kx, n.ky, n.sy];
                inst.uvt = [n.tx, n.ty, mode, aa];
            }
        };
        let tex = match op.src {
            Src::Solid(c) => {
                inst.color = [c.r * c.a, c.g * c.a, c.b * c.a, c.a];
                TexRef::White
            }
            Src::Image(pm) => {
                uv_for(pm.width(), pm.height(), MODE_TEXTURE, &mut inst);
                TexRef::Static(pm)
            }
            Src::Frame { key, ver, data, w, h, fmt, adjust } => {
                uv_for(w, h, MODE_YUV, &mut inst);
                inst.vid = fmt.shader_params();
                inst.adj = adjust;
                TexRef::Video { key, ver, data, w, h, fmt }
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

/// A video layer's three plane textures, updated in place every frame.
struct VideoTex {
    ver: i64,
    planes: [wgpu::Texture; 3],
    bg: Arc<wgpu::BindGroup>,
    w: u32,
    h: u32,
    fmt: FrameFormat,
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
    /// 16-bit normalized textures available (10-bit video at full precision)
    pub high_bit_video: bool,
    pub name: String,
    alive: Arc<AtomicBool>,
}

impl Drop for Gpu {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

/// Blocking `device.poll(Wait)` holds wgpu's device lock while waiting, which would
/// serialize all workers. Instead one thread polls without blocking and the
/// `map_async` callbacks wake the workers.
fn spawn_poller(device: wgpu::Device, alive: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("flits-gpu-poll".into())
        .spawn(move || {
            while alive.load(Ordering::Relaxed) {
                let pause = match device.poll(wgpu::PollType::Poll) {
                    Ok(status) if status.is_queue_empty() => Duration::from_micros(500),
                    Ok(_) => Duration::from_micros(50),
                    Err(_) => Duration::from_millis(1),
                };
                std::thread::sleep(pause);
            }
        })
        .expect("spawning GPU poll thread");
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
    view: wgpu::TextureView,
    uni_bg: wgpu::BindGroup,
    inst: wgpu::Buffer,
    inst_cap: usize,
    yuv_buf: wgpu::Buffer,
    yuv_bg: wgpu::BindGroup,
    read_buf: wgpu::Buffer,
    rgba_buf: Option<wgpu::Buffer>,
    videos: HashMap<usize, VideoTex>,
}

fn texture_entry(binding: u32, stage: wgpu::ShaderStages, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn buffer_entry(binding: u32, stage: wgpu::ShaderStages, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn make_texture(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_texture(queue: &wgpu::Queue, tex: &wgpu::Texture, data: &[u8], w: u32, h: u32, bytes_per_px: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_px * w), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    views: [&wgpu::TextureView; 3],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(views[0]) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(views[1]) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(views[2]) },
        ],
    })
}

fn rgba_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    data: &[u8],
) -> wgpu::BindGroup {
    let tex = make_texture(device, w, h, wgpu::TextureFormat::Rgba8Unorm);
    write_texture(queue, &tex, data, w, h, 4);
    let view = tex.create_view(&Default::default());
    make_bind_group(device, layout, sampler, [&view, &view, &view])
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
        let high_bit_video = adapter.features().contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM);
        let features = if high_bit_video { wgpu::Features::TEXTURE_FORMAT_16BIT_NORM } else { wgpu::Features::empty() };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("flits"),
            required_features: features,
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .context("creating GPU device")?;

        let render_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_WGSL.into()),
        });
        let yuv_mod = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("yuv"), source: wgpu::ShaderSource::Wgsl(YUV_WGSL.into()) });
        let (vs, fs, cs) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::COMPUTE);

        let uni_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
            entries: &[buffer_entry(0, vs, wgpu::BufferBindingType::Uniform)],
        });
        // binding 0 = RGBA texture or Y plane, 2/3 = U/V planes (= binding 0 for non-video)
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                texture_entry(0, fs, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fs,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(2, fs, true),
                texture_entry(3, fs, true),
            ],
        });
        let yuv_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv"),
            entries: &[
                texture_entry(0, cs, false),
                buffer_entry(1, cs, wgpu::BufferBindingType::Storage { read_only: false }),
                buffer_entry(2, cs, wgpu::BufferBindingType::Uniform),
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&uni_layout, &tex_layout],
            push_constant_ranges: &[],
        });
        let attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4
        ];
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
        let alive = Arc::new(AtomicBool::new(true));
        spawn_poller(device.clone(), alive.clone());
        let white = Arc::new(rgba_bind_group(&device, &queue, &tex_layout, &sampler, 1, 1, &[255, 255, 255, 255]));
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
            high_bit_video,
            name: format!("{} ({:?})", info.name, info.backend),
            alive,
        })
    }

    fn texture(&self, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        make_texture(&self.device, w, h, format)
    }

    fn write(&self, tex: &wgpu::Texture, data: &[u8], w: u32, h: u32, bytes_per_px: u32) {
        write_texture(&self.queue, tex, data, w, h, bytes_per_px)
    }

    fn bind_group(&self, views: [&wgpu::TextureView; 3]) -> wgpu::BindGroup {
        make_bind_group(&self.device, &self.tex_layout, &self.sampler, views)
    }

    fn rgba_bind_group(&self, w: u32, h: u32, data: &[u8]) -> wgpu::BindGroup {
        rgba_bind_group(&self.device, &self.queue, &self.tex_layout, &self.sampler, w, h, data)
    }

    fn static_bg(&self, pm: &Pixmap) -> Arc<wgpu::BindGroup> {
        let key = pm as *const Pixmap as usize;
        if let Some(c) = self.statics.lock().unwrap().0.get_mut(&key) {
            c.last = Instant::now();
            return c.bg.clone();
        }
        // upload outside the lock so other workers keep going
        let bg = Arc::new(self.rgba_bind_group(pm.width(), pm.height(), pm.data()));
        let size = pm.data().len() as u64;
        let mut guard = self.statics.lock().unwrap();
        let (map, bytes) = &mut *guard;
        if *bytes + size > TEX_BUDGET {
            // evict textures nobody used in the last few seconds (workers move forward in time)
            let now = Instant::now();
            map.retain(|_, c| now.duration_since(c.last).as_secs_f32() < 3.0);
            *bytes = map.values().map(|c| c.bytes).sum();
        }
        let entry = map.entry(key).or_insert_with(|| {
            *bytes += size;
            Cached { bg: bg.clone(), bytes: size, last: Instant::now() }
        });
        entry.bg.clone()
    }

    fn video_bg(&self, tg: &mut GpuTarget, key: usize, ver: i64, data: &[u8], w: u32, h: u32, fmt: FrameFormat) -> Arc<wgpu::BindGroup> {
        let (format, bpp) = if fmt.high_bit { (wgpu::TextureFormat::R16Unorm, 2) } else { (wgpu::TextureFormat::R8Unorm, 1) };
        let stale = tg.videos.get(&key).is_none_or(|v| v.w != w || v.h != h || v.fmt.high_bit != fmt.high_bit);
        if stale {
            let planes = [self.texture(w, h, format), self.texture(w / 2, h / 2, format), self.texture(w / 2, h / 2, format)];
            let views = planes.each_ref().map(|t| t.create_view(&Default::default()));
            let bg = Arc::new(self.bind_group([&views[0], &views[1], &views[2]]));
            tg.videos.insert(key, VideoTex { ver: i64::MIN, planes, bg, w, h, fmt });
        }
        let v = tg.videos.get_mut(&key).unwrap();
        if v.ver != ver {
            let y = (w * h * bpp) as usize;
            let c = ((w / 2) * (h / 2) * bpp) as usize;
            self.write(&v.planes[0], &data[..y], w, h, bpp);
            self.write(&v.planes[1], &data[y..y + c], w / 2, h / 2, bpp);
            self.write(&v.planes[2], &data[y + c..y + 2 * c], w / 2, h / 2, bpp);
            v.ver = ver;
        }
        v.bg.clone()
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
        let inst = self.instance_buffer(inst_cap);
        GpuTarget { w, h, real_w, tex, view, uni_bg, inst, inst_cap, yuv_buf, yuv_bg, read_buf, rgba_buf: None, videos: HashMap::new() }
    }

    fn instance_buffer(&self, cap: usize) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (cap * std::mem::size_of::<Inst>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Composite the ops and read the frame back into `out` (YUV for encoding, RGBA for stills).
    pub fn render(&self, tg: &mut GpuTarget, bg: Rgba, painter: GpuPainter, mode: Readback, out: &mut Vec<u8>) -> Result<()> {
        let ops = painter.ops;
        let mut bgs: Vec<Arc<wgpu::BindGroup>> = Vec::with_capacity(ops.len());
        let mut used_videos: Vec<usize> = Vec::new();
        for (_, t) in &ops {
            bgs.push(match t {
                TexRef::White => self.white.clone(),
                TexRef::Static(pm) => self.static_bg(pm),
                TexRef::Video { key, ver, data, w, h, fmt } => {
                    used_videos.push(*key);
                    self.video_bg(tg, *key, *ver, data, *w, *h, *fmt)
                }
            });
        }
        // video layers that ended release their textures
        tg.videos.retain(|k, _| used_videos.contains(k));

        if ops.len() > tg.inst_cap {
            tg.inst_cap = ops.len().next_power_of_two();
            tg.inst = self.instance_buffer(tg.inst_cap);
        }
        if !ops.is_empty() {
            let insts: Vec<Inst> = ops.iter().map(|(i, _)| *i).collect();
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
            // one instanced draw per run of quads sharing a texture (solid fills all share `white`)
            let mut i = 0;
            while i < bgs.len() {
                let mut j = i + 1;
                while j < bgs.len() && Arc::ptr_eq(&bgs[i], &bgs[j]) {
                    j += 1;
                }
                pass.set_bind_group(1, bgs[i].as_ref(), &[]);
                pass.draw(0..6, i as u32..j as u32);
                i = j;
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
                let buf = tg.rgba_buf.get_or_insert_with(|| {
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: (row * tg.h) as u64,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                });
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
        self.queue.submit([enc.finish()]);

        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        // the poller thread drives the callback
        rx.recv().map_err(|e| anyhow!("GPU readback: {e}"))?.map_err(|e| anyhow!("GPU readback failed: {e}"))?;
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
