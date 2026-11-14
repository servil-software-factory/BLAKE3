# `blake3-servil-core` 0.9 (API Preview)

*0.9 is unreleased ([release plan](https://github.com/servil-software-factory/BLAKE3/issues/357)). Signatures below reflect current code and may change before release, when full rustdoc will publish at <https://servil-software-factory.github.io/BLAKE3/>.*

`blake3-servil-core` is a fast, single-threaded BLAKE3 library for 64-bit Arm (Linux, macOS) and x86-64 (Linux). Every call runs on the calling thread, spawns zero background threads, and produces the same digests as official BLAKE3. Every assembly kernel is formally proved against the BLAKE3 specification; the surrounding Rust is tested.

## Using the API

Call `initialize()` once at startup before hashing. Hashing functions live in two modules:

- `convenience` takes ordinary `&[u8]` slices and is tuned for occasional calls amid other work.
- `throughput` takes 64-byte-aligned `Block`s and batches, writes into caller-supplied buffers, and is tuned for sustained back-to-back calls.

Disable default features (`default-features = false`) for an allocator-free `no_std` build. Optional `serde` and `zeroize` features add trait support.

`blake3-servil` re-exports this crate and adds multithreading via a shared thread pool. [`capi/b3.h`](../capi/b3.h) provides the convenience API for C and C++.

## Crate root

```rust
const BUILD_VERSION  (from `blake3_servil_version`)
const OUT_LEN: usize
const KEY_LEN: usize
const BLOCK_LEN: usize
const CHUNK_LEN: usize
```


- `fn initialize()`  
  Run the startup self-test and detect the CPU.


### `struct Hash`

An output of the default size, 32 bytes, which provides constant-time equality checking.

```rust
const fn as_bytes(&self) -> &[u8; 32]
const fn from_bytes(bytes: [u8; 32]) -> Self
const fn as_slice(&self) -> &[u8]
fn from_slice(bytes: &[u8]) -> Result<Self, TryFromSliceError>
fn to_hex(&self) -> ArrayString<{ _ }>
fn from_hex(hex: impl AsRef<[u8]>) -> Result<Self, HexError>
```

Implements: `Clone`, `Copy`, `Debug`, `Deserialize`, `Display`, `Eq`, `From`, `FromStr`, `Hash`, `PartialEq`, `Serialize`, `Zeroize`.


### `struct HexError`

The error type for `Hash::from_hex`.

Implements: `Clone`, `Debug`, `Display`, `Error`.


### `type_alias ContextKey`

An alias to distinguish `context_key` outputs from other keys.


### `enum Mode<'a>`

Which of BLAKE3's three functions a call computes.

```rust
Plain
Keyed(&'a [u8; 32])
DeriveKey(&'a str)
DeriveKeyWith(&'a ContextKey)
```

Implements: `Clone`, `Copy`.


### `struct OutputReader`

An incremental reader for extended output, returned by `Hasher::finalize_xof`.

```rust
fn fill(&mut self, buf: &mut [u8])
fn position(&self) -> u64
fn set_position(&mut self, position: u64)
```

Implements: `Clone`, `Debug`, `Read`, `Seek`, `Zeroize`.


## Module `convenience`

The convenience API.


- `fn hash(input: &[u8]) -> Hash`  
  The hash of `input`, on the calling thread.
- `fn hash_with(mode: Mode<'_>, input: &[u8]) -> Hash`  
  `hash` in any `Mode`.
- `fn hash_xof(mode: Mode<'_>, input: &[u8]) -> OutputReader`  
  The extended output of `input` in `mode`.


### `struct Hasher`

A message's hash, fed in pieces.

```rust
fn new(mode: Mode<'_>, length_hint: Option<u64>) -> Self
fn update(&mut self, input: &[u8]) -> &mut Self
fn finalize(&self) -> Hash
fn finalize_xof(&self) -> OutputReader
fn update_reader(&mut self, reader: impl Read) -> Result<&mut Self>
```

Implements: `Clone`, `Debug`, `Write`, `Zeroize`.


## Module `throughput`

The throughput API, for power users.


- `fn finalize_each(hashers: &mut [Hasher], which: &[usize], out: &mut [[u8; 32]])`  
  The hash of each message `which` names.
- `fn hash_each(mode: Mode<'_>, items: &[(&[Block], usize)], out: &mut [[u8; 32]])`  
  The hash of each message, of any lengths, on the calling thread.
- `fn update_each(hashers: &mut [Hasher], pieces: &[(usize, &[u8])])`  
  Feed each piece to its hasher.
- `fn hash(mode: Mode<'_>, input: &[Block], len: usize) -> Hash`  
  The hash in `mode` of the message of `len` bytes that starts `input`.
- `fn hash_many<P: Prepared>(mode: Mode<'_>, batch: &P, out: &mut [[u8; 32]])`  
  The hashes of a `Batch`'s messages, in `mode`, on the calling thread.
- `fn context_key(context: &str) -> ContextKey`  
  Hash a key-derivation context string and return a `ContextKey`.
- `const fn left_len(len: u64) -> u64`  
  The bytes of the left subtree of a message, or of a subtree, of `len` bytes.


### `struct Packed<const N: usize>`

`count` messages of `N` bytes each, packed end to end with nothing between them, on a 128-byte boundary.

```rust
fn new(count: usize) -> Self
fn as_blocks(&self) -> &[Block]
```

Implements: `Deref`, `DerefMut`, `Prepared`.


### `struct Batch<'a>`

Messages of one length in `Block`s, the batch calls' input.

```rust
fn new(message_len: usize, count: usize) -> Self
fn from_blocks(blocks: &'a [Block], message_len: usize) -> Self
fn message_len(&self) -> usize
fn len(&self) -> usize
fn is_empty(&self) -> bool
fn blocks(&self) -> &[Block]
fn message_mut(&mut self, i: usize) -> &mut [u8]
fn set(&mut self, i: usize, message: &[u8])
fn blocks_mut(&mut self) -> &mut [Block]
fn digests_mut(&mut self) -> &mut [[u8; 32]]
```

Implements: `Clone`, `Prepared`.


### `struct Block`

64 bytes on a 64-byte boundary.

```rust
const ZERO
fn bytes(blocks: &[Block]) -> &[u8]
fn bytes_mut(blocks: &mut [Block]) -> &mut [u8]
fn from_bytes(bytes: &[u8]) -> Option<&[Block]>
```

Implements: `Clone`, `Copy`, `Debug`, `Eq`, `PartialEq`.


### `trait Prepared`

The batch calls' input.


### `struct Cv`

A subtree's chaining value.

```rust
const fn as_bytes(&self) -> &[u8; 32]
const fn from_bytes(bytes: [u8; 32]) -> Self
```

Implements: `Clone`, `Copy`, `Debug`, `Eq`, `PartialEq`.


### `struct TreeHasher`

A `Mode` prepared for a program that builds or verifies a BLAKE3 tree itself (verified streaming, an outboard, a tree hashed in parts on other machines).

```rust
fn new(mode: Mode<'_>) -> Self
fn on_this_core(self) -> Self
fn subtree_cv(&self, first_chunk: u64, input: &[u8]) -> Cv
fn subtree_halves(&self, first_chunk: u64, input: &[u8]) -> [Cv; 2]
fn parent_cv(&self, left: &Cv, right: &Cv) -> Cv
fn parent_cvs(&self, children: &[Cv], out: &mut [Cv])
fn root_hash(&self, left: &Cv, right: &Cv) -> Hash
fn root_output(&self, left: &Cv, right: &Cv) -> OutputReader
```

Implements: `Clone`, `Zeroize`.
