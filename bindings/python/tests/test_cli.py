"""python bindings/python/tests/test_cli.py (no dependencies; the flits CLI itself is not run)"""

import json
import sys
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import flits  # noqa: E402
from flits import _cli  # noqa: E402


def res(p: str) -> str:
    return str(Path(p).resolve())


class ToArgs(unittest.TestCase):
    def test_flags(self) -> None:
        self.assertEqual(
            flits.to_args({"keep_temp": True, "workers": 4, "quiet": False, "at": [1, 2]}),
            ["--keep-temp", "--workers", "4", "--at", "1", "--at", "2"],
        )


class ReelArgs(unittest.TestCase):
    def test_mirrors_the_cli_flags(self) -> None:
        args = flits.reel_args(
            "in.mp4", "out/reel.mp4", from_="1:12", to=100, fillers=True, no_punch=True, logo="logo.png", end="Volg ons", end_duration=2.5
        )
        self.assertEqual(args[:4], ["reel", res("in.mp4"), "-o", res("out/reel.mp4")])
        self.assertEqual(
            args[4:],
            ["--from", "1:12", "--to", "100", "--fillers", "--no-punch", "--logo", res("logo.png"), "--end", "Volg ons", "--end-duration", "2.5"],
        )

    def test_clips_auto_end_logo_and_pick(self) -> None:
        self.assertEqual(flits.reel_args("in.mp4", clips=[("0:40", "1:05"), (190, 212), "5:00-5:20"])[2:], ["--clips", "0:40-1:05,190-212,5:00-5:20"])
        self.assertEqual(flits.reel_args("in.mp4", auto=True, candidates=3)[2:], ["--auto", "--candidates", "3"])
        self.assertEqual(flits.reel_args("in.mp4", auto=30)[2:], ["--auto", "30"])
        self.assertEqual(flits.reel_args("in.mp4", end_logo=True)[2:], ["--end-logo"])
        self.assertEqual(flits.reel_args("in.mp4", end_logo="mark.svg")[2:], ["--end-logo", res("mark.svg")])
        self.assertEqual(flits.reel_args("in.mp4", pick=True, auto=60)[2:], ["--pick", "--auto", "60", "--json"])


class Reel(unittest.TestCase):
    def test_returns_the_written_videos(self) -> None:
        with mock.patch.object(_cli, "run", return_value="/tmp/a-1.mp4\n/tmp/a-2.mp4\n") as run:
            out = flits.reel("in.mp4", "a.mp4", auto=True, candidates=2)
        self.assertEqual(out, [Path("/tmp/a-1.mp4"), Path("/tmp/a-2.mp4")])
        self.assertEqual(run.call_args.args[0][:2], ["reel", res("in.mp4")])

    def test_pick_returns_the_candidates(self) -> None:
        cands = [{"rank": 1, "from": 12.5, "to": 55.0, "length": 41.0, "score": 3.5, "first": "Why?", "last": "So.", "reasons": []}]
        with mock.patch.object(_cli, "run", return_value=json.dumps(cands)):
            self.assertEqual(flits.reel("in.mp4", pick=True), cands)


if __name__ == "__main__":
    unittest.main()
