//! Minimal linked consumer of `blake3-servil` and `blake3-servil-core`
//! compiled by `tools/stack_budget.py` with `-Z emit-stack-sizes` so every
//! hot generic monomorphization (`Pipeline<T, B, P>::new`, `with_owners`,
//! `with_batches`, `message`, `message_with`, `room`, `commit`, `owned`,
//! `batch`, `end`, `abort`, `ready`, `next_event`, cancellation / `Drop`,
//! `Hasher`, `update_multithreaded`, `hash_multithreaded`,
//! `hash_many_multithreaded`) is emitted and budgeted inside small
//! `#[inline(never)]` consumer wrappers alongside the library `.rlib`s.
//! Each wrapper keeps one cap regime (`core`, `core::Hasher`, or `threads`)
//! so `#[inline(always)]` library methods cannot hide oversized frames in
//! consumer stack frames.

use std::hint::black_box;

use blake3_servil::throughput::{Batch, Block, Limits, Pipeline, block_on, hash_many_multithreaded, hash_multithreaded, update_multithreaded};
use blake3_servil::{Mode, convenience};

#[inline(never)]
fn probe_threads_initialize() {
    blake3_servil::initialize_multithreaded(None);
}

#[inline(never)]
fn probe_core_batch_prepare(buf: &[u8]) -> Batch<'static> {
    let mut batch = Batch::new(buf.len(), 2);
    batch.set(0, buf);
    batch.set(1, buf);
    batch
}

#[inline(never)]
fn probe_core_hasher_update(buf: &[u8]) -> convenience::Hasher {
    let mut h = convenience::Hasher::new(Mode::Plain, None);
    h.update(buf);
    h
}

#[inline(never)]
fn probe_core_hasher_finalize(h: &convenience::Hasher) -> ([u8; 32], [u8; 64]) {
    let digest = *h.finalize().as_bytes();
    let mut xof = [0u8; 64];
    h.finalize_xof().fill(&mut xof);
    (digest, xof)
}

#[inline(never)]
fn probe_threads_update_mt(h: &mut convenience::Hasher, buf: &[u8]) {
    update_multithreaded(h, buf);
}

#[inline(never)]
fn probe_threads_one_shot(blocks: &[Block], batch: &Batch<'_>) -> ([u8; 32], [[u8; 32]; 2]) {
    let first = *hash_multithreaded(Mode::Plain, blocks, blocks.len() * 64).as_bytes();
    let mut outs = [[0u8; 32]; 2];
    hash_many_multithreaded(Mode::Plain, batch, &mut outs);
    (first, outs)
}

#[inline(never)]
fn probe_threads_pipeline_rooms(buf: &[u8]) {
    let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 8, rooms: 4 });
    block_on(async {
        let mut msg = pipe.message(0usize, Some(buf.len() as u64)).await;
        let mut room = pipe.room().await;
        room.bytes_mut()[..buf.len()].copy_from_slice(buf);
        pipe.commit(&mut msg, &mut room, buf.len());
        pipe.end(msg);
        pipe.ready().await;
        black_box(pipe.next_event());
    });
}

#[inline(never)]
fn probe_threads_pipeline_owned(owner: Vec<u8>) {
    let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 0 });
    block_on(async {
        let len = owner.len() as u64;
        let msg = pipe.message(1usize, Some(len)).await;
        pipe.owned(msg, owner);
        pipe.ready().await;
        black_box(pipe.next_event());
    });
}

#[inline(never)]
fn probe_threads_pipeline_batch(batch: Batch<'static>) {
    let mut pipe = Pipeline::<usize, (), Batch<'static>>::with_batches(Mode::Plain, Limits { messages: 4, rooms: 0 });
    block_on(async {
        let msg = pipe.message(2usize, None).await;
        let digests = vec![[0u8; 32]; batch.len()].into_boxed_slice();
        pipe.batch(msg, batch, digests);
        pipe.ready().await;
        black_box(pipe.next_event());
    });
}

#[inline(never)]
fn probe_threads_pipeline_message_with(key: &[u8; 32], buf: &[u8]) {
    let mut pipe = Pipeline::<usize, ()>::new(Mode::Plain, Limits { messages: 4, rooms: 2 });
    block_on(async {
        let mut msg = pipe.message_with(Mode::Keyed(key), 3usize, Some(buf.len() as u64)).await;
        let mut room = pipe.room().await;
        room.bytes_mut()[..buf.len()].copy_from_slice(buf);
        pipe.commit(&mut msg, &mut room, buf.len());
        pipe.end(msg);
        pipe.ready().await;
        black_box(pipe.next_event());
    });
}

#[inline(never)]
fn probe_threads_pipeline_cancel(buf: &[u8], owner: Vec<u8>) {
    // Explicit abort on both an uncommitted message and a room-committed message,
    // plus dropping the Pipeline while an owned message, an ended room-committed
    // message, and a surviving Room are still held (exercising both Pipeline::drop
    // and Room::drop's closing path after Pipeline teardown).
    let mut pipe = Pipeline::<usize, Vec<u8>>::with_owners(Mode::Plain, Limits { messages: 8, rooms: 4 });
    let surviving_room = block_on(async {
        let msg_uncommitted = pipe.message(10usize, Some(buf.len() as u64)).await;
        pipe.abort(msg_uncommitted);

        let mut msg_committed = pipe.message(11usize, Some(buf.len() as u64)).await;
        let mut room = pipe.room().await;
        room.bytes_mut()[..buf.len()].copy_from_slice(buf);
        pipe.commit(&mut msg_committed, &mut room, buf.len());
        pipe.abort(msg_committed);

        let msg_owned = pipe.message(12usize, Some(owner.len() as u64)).await;
        pipe.owned(msg_owned, owner);
        let mut msg_ended = pipe.message(13usize, Some(buf.len() as u64)).await;
        let mut room2 = pipe.room().await;
        room2.bytes_mut()[..buf.len()].copy_from_slice(buf);
        pipe.commit(&mut msg_ended, &mut room2, buf.len());
        pipe.end(msg_ended);
        pipe.room().await
    });
    drop(pipe);
    drop(surviving_room);
}

fn main() {
    probe_threads_initialize();

    let buf = black_box([0x5Au8; 64]);
    let key = black_box([0xA5u8; 32]);
    let blocks = black_box([Block::ZERO; 4]);

    let mut hasher = probe_core_hasher_update(black_box(&buf));
    probe_threads_update_mt(black_box(&mut hasher), black_box(&buf));
    black_box(probe_core_hasher_finalize(black_box(&hasher)));

    let one_shot_batch = probe_core_batch_prepare(black_box(&buf));
    black_box(probe_threads_one_shot(black_box(&blocks), black_box(&one_shot_batch)));

    probe_threads_pipeline_rooms(black_box(&buf));
    probe_threads_pipeline_owned(black_box(buf.to_vec()));

    let batch = probe_core_batch_prepare(black_box(&buf));
    probe_threads_pipeline_batch(black_box(batch));

    probe_threads_pipeline_message_with(black_box(&key), black_box(&buf));
    probe_threads_pipeline_cancel(black_box(&buf), black_box(buf.to_vec()));
}
