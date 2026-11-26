//! Minimal linked consumer of `blake3-servil` and `blake3-servil-core`
//! compiled by `tools/stack_budget.py` with `-Z emit-stack-sizes` so every
//! hot generic monomorphization (`Pipeline<T, B, P>::new`, `message`, `room`,
//! `commit`, `end`, `ready`, `next_event`, `Hasher`, `hash_multithreaded`,
//! `hash_many_multithreaded`) is emitted and checked against the stack-frame
//! caps alongside the library `.rlib`s.

use std::hint::black_box;

use blake3_servil::throughput::{Batch, Block, Limits, Pipeline, block_on, hash_many_multithreaded, hash_multithreaded, update_multithreaded};
use blake3_servil::{Mode, convenience};

fn main() {
    blake3_servil::initialize_multithreaded(None);

    let buf = black_box([0x5Au8; 64]);
    let mut h = convenience::Hasher::new(Mode::Plain, None);
    h.update(&buf);
    black_box(h.finalize());
    update_multithreaded(&mut h, &buf);
    let mut xof = [0u8; 64];
    h.finalize_xof().fill(&mut xof);
    black_box(xof);

    let blocks = [Block::ZERO; 4];
    black_box(hash_multithreaded(Mode::Plain, &blocks, 256));

    let mut batch = Batch::new(64, 2);
    batch.set(0, &buf);
    batch.set(1, &buf);
    let mut outs = [[0u8; 32]; 2];
    hash_many_multithreaded(Mode::Plain, &batch, &mut outs);
    black_box(outs);

    let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 8, rooms: 4 });
    block_on(async {
        let mut msg = pipe.message(0usize, Some(buf.len() as u64)).await;
        let mut room = pipe.room().await;
        room.bytes_mut()[..buf.len()].copy_from_slice(&buf);
        pipe.commit(&mut msg, &mut room, buf.len());
        pipe.end(msg);
        pipe.ready().await;
        black_box(pipe.next_event());
    });
}
