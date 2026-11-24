#!/usr/bin/env python3
"""Independent pure-Python BLAKE3 reference implementation for release tools
(BLAKE3 #446).

Used by `tools/b3sum-release.sh`, `tools/crates-release.sh`, and
`tools/release_artifacts.py` so release `.b3` digests are computed and verified
independently of the `b3sum` binary under check.

CLI usage (same two-space output format as `b3sum` and `sha256sum`):

    python3 tools/blake3_ref.py FILE...
"""

import struct
import sys

IV = (
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
)

CHUNK_START = 1 << 0
CHUNK_END = 1 << 1
PARENT = 1 << 2
ROOT = 1 << 3

BLOCK_LEN = 64
CHUNK_LEN = 1024

_U32_16 = struct.Struct("<16I")
_U32_8 = struct.Struct("<8I")
_M = 0xFFFFFFFF


def _compress(cv, block, block_len, counter, flags):
    """BLAKE3 compression function returning the first 8 output words (chaining value)."""
    m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15 = _U32_16.unpack(block)
    h0, h1, h2, h3, h4, h5, h6, h7 = cv
    v0, v1, v2, v3, v4, v5, v6, v7 = h0, h1, h2, h3, h4, h5, h6, h7
    v8, v9, v10, v11 = 0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A
    v12 = counter & _M
    v13 = (counter >> 32) & _M
    v14 = block_len
    v15 = flags

    for r in range(7):
        # Column step
        v0 = (v0 + v4 + m0) & _M; t = v12 ^ v0; v12 = ((t >> 16) | (t << 16)) & _M
        v8 = (v8 + v12) & _M; t = v4 ^ v8; v4 = ((t >> 12) | (t << 20)) & _M
        v0 = (v0 + v4 + m1) & _M; t = v12 ^ v0; v12 = ((t >> 8) | (t << 24)) & _M
        v8 = (v8 + v12) & _M; t = v4 ^ v8; v4 = ((t >> 7) | (t << 25)) & _M

        v1 = (v1 + v5 + m2) & _M; t = v13 ^ v1; v13 = ((t >> 16) | (t << 16)) & _M
        v9 = (v9 + v13) & _M; t = v5 ^ v9; v5 = ((t >> 12) | (t << 20)) & _M
        v1 = (v1 + v5 + m3) & _M; t = v13 ^ v1; v13 = ((t >> 8) | (t << 24)) & _M
        v9 = (v9 + v13) & _M; t = v5 ^ v9; v5 = ((t >> 7) | (t << 25)) & _M

        v2 = (v2 + v6 + m4) & _M; t = v14 ^ v2; v14 = ((t >> 16) | (t << 16)) & _M
        v10 = (v10 + v14) & _M; t = v6 ^ v10; v6 = ((t >> 12) | (t << 20)) & _M
        v2 = (v2 + v6 + m5) & _M; t = v14 ^ v2; v14 = ((t >> 8) | (t << 24)) & _M
        v10 = (v10 + v14) & _M; t = v6 ^ v10; v6 = ((t >> 7) | (t << 25)) & _M

        v3 = (v3 + v7 + m6) & _M; t = v15 ^ v3; v15 = ((t >> 16) | (t << 16)) & _M
        v11 = (v11 + v15) & _M; t = v7 ^ v11; v7 = ((t >> 12) | (t << 20)) & _M
        v3 = (v3 + v7 + m7) & _M; t = v15 ^ v3; v15 = ((t >> 8) | (t << 24)) & _M
        v11 = (v11 + v15) & _M; t = v7 ^ v11; v7 = ((t >> 7) | (t << 25)) & _M

        # Diagonal step
        v0 = (v0 + v5 + m8) & _M; t = v15 ^ v0; v15 = ((t >> 16) | (t << 16)) & _M
        v10 = (v10 + v15) & _M; t = v5 ^ v10; v5 = ((t >> 12) | (t << 20)) & _M
        v0 = (v0 + v5 + m9) & _M; t = v15 ^ v0; v15 = ((t >> 8) | (t << 24)) & _M
        v10 = (v10 + v15) & _M; t = v5 ^ v10; v5 = ((t >> 7) | (t << 25)) & _M

        v1 = (v1 + v6 + m10) & _M; t = v12 ^ v1; v12 = ((t >> 16) | (t << 16)) & _M
        v11 = (v11 + v12) & _M; t = v6 ^ v11; v6 = ((t >> 12) | (t << 20)) & _M
        v1 = (v1 + v6 + m11) & _M; t = v12 ^ v1; v12 = ((t >> 8) | (t << 24)) & _M
        v11 = (v11 + v12) & _M; t = v6 ^ v11; v6 = ((t >> 7) | (t << 25)) & _M

        v2 = (v2 + v7 + m12) & _M; t = v13 ^ v2; v13 = ((t >> 16) | (t << 16)) & _M
        v8 = (v8 + v13) & _M; t = v7 ^ v8; v7 = ((t >> 12) | (t << 20)) & _M
        v2 = (v2 + v7 + m13) & _M; t = v13 ^ v2; v13 = ((t >> 8) | (t << 24)) & _M
        v8 = (v8 + v13) & _M; t = v7 ^ v8; v7 = ((t >> 7) | (t << 25)) & _M

        v3 = (v3 + v4 + m14) & _M; t = v14 ^ v3; v14 = ((t >> 16) | (t << 16)) & _M
        v9 = (v9 + v14) & _M; t = v4 ^ v9; v4 = ((t >> 12) | (t << 20)) & _M
        v3 = (v3 + v4 + m15) & _M; t = v14 ^ v3; v14 = ((t >> 8) | (t << 24)) & _M
        v9 = (v9 + v14) & _M; t = v4 ^ v9; v4 = ((t >> 7) | (t << 25)) & _M

        if r < 6:
            # Message permutation: [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]
            m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11, m12, m13, m14, m15 = (
                m2, m6, m3, m10, m7, m0, m4, m13, m1, m11, m12, m5, m9, m14, m15, m8,
            )

    return (
        v0 ^ v8, v1 ^ v9, v2 ^ v10, v3 ^ v11,
        v4 ^ v12, v5 ^ v13, v6 ^ v14, v7 ^ v15,
    )


def _chunk_output(chunk, counter):
    """Compress all blocks of `chunk` (0..=1024 bytes) before its final output."""
    cv = IV
    n = len(chunk)
    offset = 0
    block_flags = CHUNK_START
    while n - offset > BLOCK_LEN:
        cv = _compress(cv, chunk[offset : offset + BLOCK_LEN], BLOCK_LEN, counter, block_flags)
        offset += BLOCK_LEN
        block_flags = 0
    tail = bytes(chunk[offset:])
    padded = tail + b"\x00" * (BLOCK_LEN - len(tail))
    return cv, padded, len(tail), counter, block_flags | CHUNK_END


def _parent_output(left_cv, right_cv):
    block = _U32_8.pack(*left_cv) + _U32_8.pack(*right_cv)
    return IV, block, BLOCK_LEN, 0, PARENT


class Hasher:
    """Streaming standard BLAKE3 hasher (32-byte digest)."""

    def __init__(self):
        self._stack = []
        self._buf = bytearray()
        self._chunk_counter = 0

    def _add_chunk_cv(self, cv, total_chunks):
        while (total_chunks & 1) == 0:
            left = self._stack.pop()
            cv = _compress(*_parent_output(left, cv))
            total_chunks >>= 1
        self._stack.append(cv)

    def update(self, data):
        view = memoryview(data).cast("B")
        pos = 0
        n = len(view)
        if self._buf:
            # Flush a full buffered chunk only once more bytes arrive so the
            # final chunk stays in `_buf` for ROOT finalization.
            if len(self._buf) == CHUNK_LEN and n > 0:
                cv = _compress(*_chunk_output(self._buf, self._chunk_counter))
                self._chunk_counter += 1
                self._add_chunk_cv(cv, self._chunk_counter)
                self._buf.clear()
            need = CHUNK_LEN - len(self._buf)
            take = min(need, n)
            self._buf.extend(view[:take])
            pos += take
        while n - pos > CHUNK_LEN:
            if self._buf:
                cv = _compress(*_chunk_output(self._buf, self._chunk_counter))
                self._chunk_counter += 1
                self._add_chunk_cv(cv, self._chunk_counter)
                self._buf.clear()
            cv = _compress(*_chunk_output(view[pos : pos + CHUNK_LEN], self._chunk_counter))
            self._chunk_counter += 1
            self._add_chunk_cv(cv, self._chunk_counter)
            pos += CHUNK_LEN
        if pos < n:
            if len(self._buf) == CHUNK_LEN:
                cv = _compress(*_chunk_output(self._buf, self._chunk_counter))
                self._chunk_counter += 1
                self._add_chunk_cv(cv, self._chunk_counter)
                self._buf.clear()
            self._buf.extend(view[pos:])

    def digest(self):
        out = _chunk_output(self._buf, self._chunk_counter)
        for left in reversed(self._stack):
            right_cv = _compress(*out)
            out = _parent_output(left, right_cv)
        cv, block, block_len, counter, flags = out
        root = _compress(cv, block, block_len, counter, flags | ROOT)
        return _U32_8.pack(*root)

    def hexdigest(self):
        return self.digest().hex()


def blake3_bytes(data):
    h = Hasher()
    h.update(data)
    return h.digest()


def blake3_hex(data):
    return blake3_bytes(data).hex()


def blake3_file_hex(path):
    h = Hasher()
    with open(path, "rb") as f:
        while True:
            piece = f.read(65536)
            if not piece:
                break
            h.update(piece)
    return h.hexdigest()


def main(argv):
    if not argv:
        sys.exit("usage: python3 tools/blake3_ref.py FILE...")
    for path in argv:
        try:
            digest = blake3_file_hex(path)
        except OSError as e:
            sys.exit(f"blake3_ref: {path}: {e}")
        print(f"{digest}  {path}")


if __name__ == "__main__":
    main(sys.argv[1:])
