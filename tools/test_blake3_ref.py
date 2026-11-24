"""Tests for tools/blake3_ref.py against the official BLAKE3 test vectors."""

import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import blake3_ref

ROOT = Path(__file__).resolve().parent.parent
VECTORS = ROOT / "test_vectors" / "test_vectors.json"


class Blake3RefTests(unittest.TestCase):
    def test_official_vectors(self):
        with open(VECTORS, encoding="utf-8") as f:
            data = json.load(f)
        cases = data["cases"]
        self.assertGreaterEqual(len(cases), 35)
        for case in cases:
            length = case["input_len"]
            want = case["hash"][:64]
            buf = bytes(i % 251 for i in range(length))
            self.assertEqual(blake3_ref.blake3_hex(buf), want, f"input_len={length}")

    def test_streaming_matches_one_shot_across_chunk_boundaries(self):
        for length in (0, 1, 63, 64, 65, 1023, 1024, 1025, 2048, 2049, 4096, 16384):
            buf = bytes((i * 37 + 13) & 0xFF for i in range(length))
            want = blake3_ref.blake3_hex(buf)
            for step in (1, 17, 64, 500, 1024, 3000):
                h = blake3_ref.Hasher()
                for pos in range(0, length, step):
                    h.update(buf[pos : pos + step])
                self.assertEqual(h.hexdigest(), want, f"length={length} step={step}")

    def test_file_and_cli_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            p1 = os.path.join(tmp, "empty.bin")
            p2 = os.path.join(tmp, "sample.bin")
            with open(p1, "wb") as f:
                f.write(b"")
            payload = bytes(i % 251 for i in range(4097))
            with open(p2, "wb") as f:
                f.write(payload)
            out = io.StringIO()
            with redirect_stdout(out):
                blake3_ref.main([p1, p2])
            lines = out.getvalue().splitlines()
            self.assertEqual(
                lines,
                [
                    f"af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262  {p1}",
                    f"{blake3_ref.blake3_hex(payload)}  {p2}",
                ],
            )
            with self.assertRaises(SystemExit):
                blake3_ref.main([os.path.join(tmp, "missing.bin")])
            with self.assertRaises(SystemExit):
                blake3_ref.main([])


if __name__ == "__main__":
    unittest.main()
