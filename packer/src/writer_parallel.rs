//! Optional parallel compression of deduplicated blocks. Results are written
//! to the archive in block-ID order. No index or compressor policy is
//! duplicated: frames use the same Zstd parameters and RAW decision.
use super::*;
use std::collections::VecDeque;
use std::mem::MaybeUninit;
use std::sync::{mpsc, Mutex};
use std::thread::{self, JoinHandle};
use zstd::zstd_safe::zstd_sys as sys;

const STACK_BYTES: usize = 512 * 1024;

// Static Zstd contexts cannot grow or allocate internally. Workspace addresses
// stay aligned and stable when this owner moves; no borrowed pointer is stored.
struct FixedEncoder {
    workspace: Vec<MaybeUninit<u64>>,
    frame: Vec<u8>,
}

fn zcheck(code: usize) -> Result<usize, WriterError> {
    // SAFETY: these functions accept any size_t returned by Zstd.
    if unsafe { sys::ZSTD_isError(code) } != 0 {
        return Err(limit_error(format!(
            "bounded Zstandard worker failed: {}",
            zstd::zstd_safe::get_error_name(code)
        )));
    }
    Ok(code)
}

impl FixedEncoder {
    fn new(options: &WriterOptions) -> Result<Self, WriterError> {
        let count = ZSTD_CONTEXT_MEMORY_ESTIMATE as usize / 8;
        let mut workspace = Vec::new();
        workspace
            .try_reserve_exact(count)
            .map_err(|_| limit_error("unable to reserve fixed Zstandard workspace"))?;
        workspace.resize_with(count, MaybeUninit::uninit);
        let bound = zstd::zstd_safe::compress_bound(options.chunk_size_bytes as usize);
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(bound)
            .map_err(|_| limit_error("unable to reserve worker frame"))?;
        // SAFETY: u64 storage provides the required 8-byte alignment; it is
        // writable for the declared extent and outlives the static context.
        let context = unsafe { sys::ZSTD_initStaticCCtx(workspace.as_mut_ptr().cast(), count * 8) };
        if context.is_null() {
            return Err(limit_error(
                "unable to initialize fixed Zstandard workspace",
            ));
        }
        // SAFETY: context was initialized above. No dictionaries, streaming or
        // internal Zstd workers are used (unsupported by static contexts).
        unsafe {
            for (parameter, value) in [
                (
                    sys::ZSTD_cParameter::ZSTD_c_compressionLevel,
                    options.zstd_compression_level,
                ),
                (
                    sys::ZSTD_cParameter::ZSTD_c_windowLog,
                    options.chunk_size_bytes.trailing_zeros() as i32,
                ),
                (sys::ZSTD_cParameter::ZSTD_c_contentSizeFlag, 1),
                (sys::ZSTD_cParameter::ZSTD_c_checksumFlag, 0),
                (sys::ZSTD_cParameter::ZSTD_c_dictIDFlag, 0),
            ] {
                zcheck(sys::ZSTD_CCtx_setParameter(context, parameter, value))?;
            }
        }
        Ok(Self { workspace, frame })
    }

    fn encode(&mut self, input: &[u8], options: &WriterOptions) -> Result<Encoded, WriterError> {
        cancelled(options)?;
        let started = Instant::now();
        let context = self.workspace.as_mut_ptr().cast::<sys::ZSTD_CCtx>();
        let capacity = self.frame.capacity();
        let out = self.frame.as_mut_ptr();
        // Prediction levels use less workspace than the configured level, so
        // the static context accepts them; the level is restored afterwards.
        let skip = crate::writer::predicts_low_gain(input, options, |level, slice| {
            // SAFETY: same owned context and distinct extents as the encode below.
            unsafe {
                zcheck(sys::ZSTD_CCtx_setParameter(
                    context,
                    sys::ZSTD_cParameter::ZSTD_c_compressionLevel,
                    level,
                ))?;
                zcheck(sys::ZSTD_CCtx_setPledgedSrcSize(
                    context,
                    slice.len() as u64,
                ))?;
                zcheck(sys::ZSTD_compress2(
                    context,
                    out.cast(),
                    capacity,
                    slice.as_ptr().cast(),
                    slice.len(),
                ))
            }
        })?;
        // SAFETY: owned initialized context.
        unsafe {
            zcheck(sys::ZSTD_CCtx_setParameter(
                context,
                sys::ZSTD_cParameter::ZSTD_c_compressionLevel,
                options.zstd_compression_level,
            ))?;
        }
        if skip {
            return Ok(Encoded {
                frame: Vec::new(),
                input: Vec::new(),
                elapsed: started.elapsed(),
                // SAFETY: owned initialized context.
                context_bytes: unsafe { sys::ZSTD_sizeof_CCtx(context) as u64 },
            });
        }
        // SAFETY: the initialized context remains owned by this worker. Input
        // and frame are distinct, valid extents. Set length only after success.
        let (written, context_bytes) = unsafe {
            zcheck(sys::ZSTD_CCtx_setPledgedSrcSize(
                context,
                input.len() as u64,
            ))?;
            let written = zcheck(sys::ZSTD_compress2(
                context,
                self.frame.as_mut_ptr().cast(),
                self.frame.capacity(),
                input.as_ptr().cast(),
                input.len(),
            ))?;
            if written > self.frame.capacity() {
                return Err(WriterError::new(
                    WriterErrorKind::Integrity,
                    "worker output exceeds frame capacity",
                ));
            }
            self.frame.set_len(written);
            (written, sys::ZSTD_sizeof_CCtx(context) as u64)
        };
        let elapsed = started.elapsed();
        cancelled(options)?;
        let mut frame = Vec::new();
        if crate::writer::keep_compressed(written, input.len(), options) {
            frame
                .try_reserve_exact(written)
                .map_err(|_| limit_error("unable to reserve pending frame"))?;
            frame.extend_from_slice(&self.frame);
        }
        Ok(Encoded {
            frame,
            input: Vec::new(),
            elapsed,
            context_bytes,
        })
    }
}

struct Encoded {
    frame: Vec<u8>,
    input: Vec<u8>,
    elapsed: Duration,
    context_bytes: u64,
}
struct Job {
    input: Vec<u8>,
    reply: mpsc::Sender<Result<Encoded, WriterError>>,
}
struct Pool {
    sender: Option<mpsc::SyncSender<Job>>,
    threads: Vec<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl Drop for Pool {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        // Replies are one-shot unbounded sends: teardown never waits for a
        // receiver to drain a full result channel.
        for worker in self.threads.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Pool {
    fn new(options: &WriterOptions, slots: usize) -> Result<Self, WriterError> {
        #[cfg(test)]
        let cancel_after_encode = CANCEL_DURING_COMPRESSION_TEST.with(|flag| flag.replace(false));
        let (sender, receiver) = mpsc::sync_channel::<Job>(slots);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut pool = Self {
            sender: Some(sender),
            threads: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        for index in 0..options.compression_workers {
            let mut encoder = FixedEncoder::new(options)?;
            let mut decoder = None;
            let mut scratch = Vec::new();
            let options = options.clone();
            let receiver = Arc::clone(&receiver);
            let stop = Arc::clone(&pool.stop);
            let worker = thread::Builder::new()
                .name(format!("nx-zstd-{index}"))
                .stack_size(STACK_BYTES)
                .spawn(move || {
                    loop {
                        // Only dequeue is serialized, never compression.
                        let job = receiver.lock().unwrap_or_else(|p| p.into_inner()).recv();
                        let Ok(job) = job else { break };
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let Job { input, reply } = job;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            let result = encoder.encode(&input, &options)?;
                            if !result.frame.is_empty() && options.verify_output_payloads {
                                verify_frame(&result.frame, &input, &mut decoder, &mut scratch)?;
                            }
                            #[cfg(test)]
                            if cancel_after_encode {
                                options.cancellation.cancel();
                            }
                            cancelled(&options)?;
                            Ok(result)
                        }))
                        .unwrap_or_else(|_| {
                            Err(WriterError::new(
                                WriterErrorKind::Integrity,
                                "compression worker panicked",
                            ))
                        })
                        .map(|mut encoded| {
                            // RAW decisions are written straight from the input.
                            encoded.input = input;
                            encoded
                        });
                        let _ = reply.send(result);
                    }
                })
                .map_err(WriterError::from)?;
            pool.threads.push(worker);
        }
        Ok(pool)
    }
}

/// Ordered compression pipeline feeding the final archive file. Blocks are
/// submitted in ID order and written in the same order, so offsets stay
/// strictly increasing; at most `slots` blocks are in flight.
pub(super) struct Pipeline {
    pool: Pool,
    pending: VecDeque<Pending>,
    slots: usize,
    credit: u64,
}

impl Pipeline {
    pub(super) fn new(
        options: &WriterOptions,
        budget: &mut MemoryBudget,
    ) -> Result<Self, WriterError> {
        let bound = zstd::zstd_safe::compress_bound(options.chunk_size_bytes as usize) as u64;
        // One credit covers input + pending result + bounded channel bookkeeping.
        // Reusable worker buffers and static contexts are charged separately.
        let credit = options.chunk_size_bytes as u64 + bound + 512;
        let slots = (options.compression_queue_bytes / credit).min(64) as usize;
        if slots < options.compression_workers as usize {
            return Err(invalid(
                "compression queue byte budget is too small for the worker count",
            ));
        }
        budget.charge(
            slots as u64 * credit,
            "bounded compression input and ordered results",
        )?;
        budget.charge(
            options.compression_workers as u64
                * (ZSTD_CONTEXT_MEMORY_ESTIMATE
                    + ZSTD_DECODER_MEMORY_ESTIMATE
                    + bound
                    + options.chunk_size_bytes as u64
                    + STACK_BYTES as u64
                    + 4096),
            "fixed worker contexts, reusable frames and stacks",
        )?;
        Ok(Self {
            pool: Pool::new(options, slots)?,
            pending: VecDeque::with_capacity(slots),
            slots,
            credit,
        })
    }

    pub(super) fn submit(
        &mut self,
        id: u32,
        input: Vec<u8>,
        out: &mut Output,
        blocks: &mut [BlockBuild],
        options: &WriterOptions,
        timings: &mut WriterTimings,
    ) -> Result<(), WriterError> {
        cancelled(options)?;
        if self.pending.len() == self.slots {
            let front = self.pending.pop_front().expect("full pending queue");
            emit(front, out, blocks, options, timings)?;
        }
        let (reply, receiver) = mpsc::channel();
        self.pool
            .sender
            .as_ref()
            .expect("live pool")
            .send(Job { input, reply })
            .map_err(|_| {
                WriterError::new(
                    WriterErrorKind::Integrity,
                    "compression workers disconnected",
                )
            })?;
        self.pending.push_back((id, receiver));
        timings.compression_queue_peak = timings
            .compression_queue_peak
            .max(self.pending.len() as u64 * self.credit);
        Ok(())
    }

    /// Write every pending block whose ID is at most `last`.
    pub(super) fn drain_through(
        &mut self,
        last: u32,
        out: &mut Output,
        blocks: &mut [BlockBuild],
        options: &WriterOptions,
        timings: &mut WriterTimings,
    ) -> Result<(), WriterError> {
        while self.pending.front().is_some_and(|(id, _)| *id <= last) {
            let front = self.pending.pop_front().expect("nonempty pending queue");
            emit(front, out, blocks, options, timings)?;
        }
        Ok(())
    }
}

type Pending = (u32, mpsc::Receiver<Result<Encoded, WriterError>>);
fn emit(
    (id, receiver): Pending,
    out: &mut Output,
    blocks: &mut [BlockBuild],
    options: &WriterOptions,
    timings: &mut WriterTimings,
) -> Result<(), WriterError> {
    let encoded = receiver.recv().map_err(|_| {
        WriterError::new(
            WriterErrorKind::Integrity,
            "compression result disconnected",
        )
    })??;
    cancelled(options)?;
    timings.compression += encoded.elapsed;
    timings.compression_context_peak = timings.compression_context_peak.max(encoded.context_bytes);
    let block = &mut blocks[id as usize];
    if encoded.frame.is_empty() {
        out.write_block(id, block, 0, &encoded.input, options, timings)
    } else {
        out.write_block(id, block, 1, &encoded.frame, options, timings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_context_matches_existing_encoder_at_all_supported_sizes_and_levels() {
        for chunk in VALID_CHUNK_SIZES {
            for level in [1, 3, 6, 9, 13] {
                let options = WriterOptions {
                    chunk_size_bytes: chunk,
                    zstd_compression_level: level,
                    ..Default::default()
                };
                let mut budget = MemoryBudget {
                    current: 0,
                    peak: 0,
                    limit: options.max_memory_bytes,
                };
                let mut serial = BlockEncoder::new(&options, &mut budget).unwrap();
                let mut fixed = FixedEncoder::new(&options).unwrap();
                let mut bytes = vec![0u8; chunk as usize];
                for (i, byte) in bytes.iter_mut().enumerate() {
                    *byte = ((i / 97 + i / 4096) % 251) as u8;
                }
                for input in [&bytes[..], &bytes[..bytes.len() - 7]] {
                    serial
                        .encode(input, &options, &mut WriterTimings::default())
                        .unwrap();
                    let result = fixed.encode(input, &options).unwrap();
                    assert_eq!(result.frame, serial.frame);
                }
            }
        }
    }
}
