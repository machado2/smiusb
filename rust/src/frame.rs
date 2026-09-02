use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

pub const BGRX_BYTES_PER_PIXEL: usize = 4;
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameId {
    pub generation: u64,
    pub sequence: u64,
}

impl fmt::Display for FrameId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.generation, self.sequence)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLayout {
    width: u32,
    height: u32,
    stride: usize,
    byte_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    ZeroDimension,
    DimensionTooLarge {
        width: u32,
        height: u32,
        maximum: u32,
    },
    SizeOverflow,
    FrameTooLarge {
        bytes: usize,
        maximum: usize,
    },
    PixelLength {
        expected: usize,
        actual: usize,
    },
    InconsistentLayout,
    AllocationFailed {
        bytes: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension => write!(formatter, "frame dimensions must be non-zero"),
            Self::DimensionTooLarge {
                width,
                height,
                maximum,
            } => write!(
                formatter,
                "frame {width}x{height} exceeds the {maximum} pixel dimension limit"
            ),
            Self::SizeOverflow => write!(formatter, "frame byte size overflows this platform"),
            Self::FrameTooLarge { bytes, maximum } => write!(
                formatter,
                "frame requires {bytes} bytes; safety limit is {maximum}"
            ),
            Self::PixelLength { expected, actual } => write!(
                formatter,
                "BGRx buffer has {actual} bytes; expected exactly {expected}"
            ),
            Self::InconsistentLayout => write!(formatter, "BGRx frame layout is inconsistent"),
            Self::AllocationFailed { bytes } => {
                write!(formatter, "cannot allocate {bytes} bytes for BGRx frame")
            }
        }
    }
}

impl FrameLayout {
    pub fn new(width: u32, height: u32) -> Result<Self, FrameError> {
        if width == 0 || height == 0 {
            return Err(FrameError::ZeroDimension);
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(FrameError::DimensionTooLarge {
                width,
                height,
                maximum: MAX_DIMENSION,
            });
        }

        let width = usize::try_from(width).map_err(|_| FrameError::SizeOverflow)?;
        let height = usize::try_from(height).map_err(|_| FrameError::SizeOverflow)?;
        let stride = width
            .checked_mul(BGRX_BYTES_PER_PIXEL)
            .ok_or(FrameError::SizeOverflow)?;
        let byte_len = stride.checked_mul(height).ok_or(FrameError::SizeOverflow)?;
        if byte_len > MAX_FRAME_BYTES {
            return Err(FrameError::FrameTooLarge {
                bytes: byte_len,
                maximum: MAX_FRAME_BYTES,
            });
        }

        Ok(Self {
            width: width as u32,
            height: height as u32,
            stride,
            byte_len,
        })
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    pub fn stride(self) -> usize {
        self.stride
    }

    pub fn byte_len(self) -> usize {
        self.byte_len
    }

    /// Recomputes every derived field. This is redundant for values produced
    /// by `new`, but gives unsafe FFI boundaries a defense-in-depth check.
    pub fn validate(self) -> Result<(), FrameError> {
        let canonical = Self::new(self.width, self.height)?;
        if self != canonical {
            return Err(FrameError::InconsistentLayout);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Frame {
    id: FrameId,
    layout: FrameLayout,
    pixels: Vec<u8>,
}

impl Frame {
    pub fn from_bgrx(
        id: FrameId,
        layout: FrameLayout,
        pixels: Vec<u8>,
    ) -> Result<Self, FrameError> {
        layout.validate()?;
        if pixels.len() != layout.byte_len() {
            return Err(FrameError::PixelLength {
                expected: layout.byte_len(),
                actual: pixels.len(),
            });
        }
        Ok(Self { id, layout, pixels })
    }

    /// Produces eight vertical BGRx color bars with a moving inverted square.
    /// It is deterministic for a given layout and frame id, making it useful
    /// for future capture/replay tests without a compositor or USB device.
    pub fn test_pattern(id: FrameId, layout: FrameLayout) -> Result<Self, FrameError> {
        const BARS: [[u8; 4]; 8] = [
            [255, 255, 255, 255], // white
            [0, 255, 255, 255],   // yellow
            [255, 255, 0, 255],   // cyan
            [0, 255, 0, 255],     // green
            [255, 0, 255, 255],   // magenta
            [0, 0, 255, 255],     // red
            [255, 0, 0, 255],     // blue
            [0, 0, 0, 255],       // black
        ];

        layout.validate()?;
        let width = layout.width() as usize;
        let height = layout.height() as usize;
        let square_size = width.min(height).min(64);
        let x_span = width - square_size + 1;
        let y_span = height - square_size + 1;
        let x_start = ((id.sequence % x_span as u64) * 37 % x_span as u64) as usize;
        let y_start = ((id.sequence % y_span as u64) * 23 % y_span as u64) as usize;
        let mut pixels = allocate_zeroed(layout.byte_len())?;

        for y in 0..height {
            for x in 0..width {
                let bar = (x * BARS.len()) / width;
                let mut pixel = BARS[bar];
                if (x_start..x_start + square_size).contains(&x)
                    && (y_start..y_start + square_size).contains(&y)
                {
                    pixel[0] = !pixel[0];
                    pixel[1] = !pixel[1];
                    pixel[2] = !pixel[2];
                }
                let offset = y * layout.stride() + x * BGRX_BYTES_PER_PIXEL;
                pixels[offset..offset + BGRX_BYTES_PER_PIXEL].copy_from_slice(&pixel);
            }
        }

        Self::from_bgrx(id, layout, pixels)
    }

    pub fn id(&self) -> FrameId {
        self.id
    }

    pub fn layout(&self) -> FrameLayout {
        self.layout
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn validate(&self) -> Result<(), FrameError> {
        self.layout.validate()?;
        if self.pixels.len() != self.layout.byte_len() {
            return Err(FrameError::PixelLength {
                expected: self.layout.byte_len(),
                actual: self.pixels.len(),
            });
        }
        Ok(())
    }

    pub fn checksum(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

        let mut checksum = FNV_OFFSET;
        for byte in self
            .id
            .generation
            .to_le_bytes()
            .into_iter()
            .chain(self.id.sequence.to_le_bytes())
            .chain(self.layout.width().to_le_bytes())
            .chain(self.layout.height().to_le_bytes())
            .chain(self.pixels().iter().copied())
        {
            checksum ^= u64::from(byte);
            checksum = checksum.wrapping_mul(FNV_PRIME);
        }
        checksum
    }
}

fn allocate_zeroed(bytes: usize) -> Result<Vec<u8>, FrameError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(bytes)
        .map_err(|_| FrameError::AllocationFailed { bytes })?;
    buffer.resize(bytes, 0);
    Ok(buffer)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PipelineStats {
    pub accepted: u64,
    pub dequeued: u64,
    pub dropped_capacity: u64,
    pub discarded_generation: u64,
    pub rejected_generation: u64,
    pub rejected_sequence: u64,
    pub rejected_sink_generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PipelineSnapshot {
    pub generation: u64,
    pub queued: usize,
    pub capacity: usize,
    pub stats: PipelineStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitOutcome {
    Queued,
    DroppedOldest(FrameId),
    RejectedGeneration { active: u64, received: u64 },
    RejectedSequence { last: u64, received: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineError {
    ZeroCapacity,
    GenerationNotIncreasing { active: u64, requested: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationMismatch {
    pub active: u64,
    pub received: u64,
}

impl fmt::Display for GenerationMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "stale work from generation {}; active generation is {}",
            self.received, self.active
        )
    }
}

impl fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => write!(formatter, "frame queue capacity must be non-zero"),
            Self::GenerationNotIncreasing { active, requested } => write!(
                formatter,
                "generation must increase (active {active}, requested {requested})"
            ),
        }
    }
}

struct PipelineState {
    generation: u64,
    capacity: usize,
    last_sequence: Option<u64>,
    frames: VecDeque<Frame>,
    stats: PipelineStats,
}

/// Thread-safe capture-to-encoder queue. It accepts only the active device
/// generation and monotonically increasing frame sequences. When full, the
/// oldest frame is dropped so a slow encoder cannot build unbounded latency.
pub struct FramePipeline {
    state: Mutex<PipelineState>,
}

impl FramePipeline {
    pub fn new(capacity: usize, initial_generation: u64) -> Result<Self, PipelineError> {
        if capacity == 0 {
            return Err(PipelineError::ZeroCapacity);
        }
        Ok(Self {
            state: Mutex::new(PipelineState {
                generation: initial_generation,
                capacity,
                last_sequence: None,
                frames: VecDeque::new(),
                stats: PipelineStats::default(),
            }),
        })
    }

    pub fn submit(&self, frame: Frame) -> SubmitOutcome {
        let mut state = self.lock_state();
        let id = frame.id();
        if id.generation != state.generation {
            state.stats.rejected_generation = state.stats.rejected_generation.saturating_add(1);
            return SubmitOutcome::RejectedGeneration {
                active: state.generation,
                received: id.generation,
            };
        }
        if state.last_sequence.is_some_and(|last| id.sequence <= last) {
            state.stats.rejected_sequence = state.stats.rejected_sequence.saturating_add(1);
            return SubmitOutcome::RejectedSequence {
                last: state.last_sequence.expect("checked as present"),
                received: id.sequence,
            };
        }

        let dropped = if state.frames.len() == state.capacity {
            state.stats.dropped_capacity = state.stats.dropped_capacity.saturating_add(1);
            state.frames.pop_front().map(|oldest| oldest.id())
        } else {
            None
        };
        state.last_sequence = Some(id.sequence);
        state.stats.accepted = state.stats.accepted.saturating_add(1);
        state.frames.push_back(frame);

        dropped.map_or(SubmitOutcome::Queued, SubmitOutcome::DroppedOldest)
    }

    pub fn try_take(&self) -> Option<Frame> {
        let mut state = self.lock_state();
        let frame = state.frames.pop_front();
        if frame.is_some() {
            state.stats.dequeued = state.stats.dequeued.saturating_add(1);
        }
        frame
    }

    /// Runs a short, non-blocking sink submission only if `id` still belongs
    /// to the active connection. The generation mutex remains held through the
    /// closure, so `begin_generation` cannot race between validation and the
    /// future `libusb_submit_transfer` call. Disconnect teardown must cancel
    /// any transfer submitted just before it acquires this mutex.
    pub fn with_current_generation<T>(
        &self,
        id: FrameId,
        submit: impl FnOnce() -> T,
    ) -> Result<T, GenerationMismatch> {
        let mut state = self.lock_state();
        if id.generation != state.generation {
            state.stats.rejected_sink_generation =
                state.stats.rejected_sink_generation.saturating_add(1);
            return Err(GenerationMismatch {
                active: state.generation,
                received: id.generation,
            });
        }
        Ok(submit())
    }

    /// Changes the active device generation and destroys queued frames from
    /// the previous connection before any new work can be accepted.
    pub fn begin_generation(&self, generation: u64) -> Result<usize, PipelineError> {
        let mut state = self.lock_state();
        if generation <= state.generation {
            return Err(PipelineError::GenerationNotIncreasing {
                active: state.generation,
                requested: generation,
            });
        }
        let discarded = state.frames.len();
        state.stats.discarded_generation = state
            .stats
            .discarded_generation
            .saturating_add(discarded as u64);
        state.frames.clear();
        state.generation = generation;
        state.last_sequence = None;
        Ok(discarded)
    }

    pub fn snapshot(&self) -> PipelineSnapshot {
        let state = self.lock_state();
        PipelineSnapshot {
            generation: state.generation,
            queued: state.frames.len(),
            capacity: state.capacity,
            stats: state.stats,
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, PipelineState> {
        // A panic in a future producer must not strand the USB teardown path.
        // The state invariants are restored before every public method returns.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(generation: u64, sequence: u64) -> Frame {
        Frame::test_pattern(
            FrameId {
                generation,
                sequence,
            },
            FrameLayout::new(8, 4).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn layout_and_buffer_lengths_are_checked() {
        assert_eq!(FrameLayout::new(0, 1), Err(FrameError::ZeroDimension));
        assert!(matches!(
            FrameLayout::new(MAX_DIMENSION + 1, 1),
            Err(FrameError::DimensionTooLarge { .. })
        ));
        assert!(matches!(
            FrameLayout::new(MAX_DIMENSION, MAX_DIMENSION),
            Err(FrameError::FrameTooLarge { .. })
        ));

        let layout = FrameLayout::new(2, 2).unwrap();
        assert_eq!(layout.stride, 8);
        assert!(matches!(
            Frame::from_bgrx(
                FrameId {
                    generation: 1,
                    sequence: 0
                },
                layout,
                vec![0; 15]
            ),
            Err(FrameError::PixelLength { .. })
        ));

        let inconsistent = FrameLayout {
            width: 2,
            height: 2,
            stride: 8,
            byte_len: 1,
        };
        assert_eq!(inconsistent.validate(), Err(FrameError::InconsistentLayout));
        assert!(matches!(
            Frame::from_bgrx(
                FrameId {
                    generation: 1,
                    sequence: 0
                },
                inconsistent,
                vec![0]
            ),
            Err(FrameError::InconsistentLayout)
        ));
        assert_eq!(
            allocate_zeroed(usize::MAX),
            Err(FrameError::AllocationFailed { bytes: usize::MAX })
        );
    }

    #[test]
    fn pattern_is_bgrx_and_changes_deterministically() {
        let layout = FrameLayout::new(800, 100).unwrap();
        let first = Frame::test_pattern(
            FrameId {
                generation: 1,
                sequence: 0,
            },
            layout,
        )
        .unwrap();
        let second = Frame::test_pattern(
            FrameId {
                generation: 1,
                sequence: 1,
            },
            layout,
        )
        .unwrap();

        let yellow_offset = 99 * layout.stride + 150 * BGRX_BYTES_PER_PIXEL;
        assert_eq!(
            &first.pixels()[yellow_offset..yellow_offset + 4],
            &[0, 255, 255, 255]
        );
        assert!(first.pixels().chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_ne!(first.checksum(), second.checksum());
        assert_eq!(first.checksum(), first.checksum());
    }

    #[test]
    fn bounded_queue_drops_oldest() {
        let pipeline = FramePipeline::new(2, 7).unwrap();
        assert_eq!(pipeline.submit(frame(7, 0)), SubmitOutcome::Queued);
        assert_eq!(pipeline.submit(frame(7, 1)), SubmitOutcome::Queued);
        assert_eq!(
            pipeline.submit(frame(7, 2)),
            SubmitOutcome::DroppedOldest(FrameId {
                generation: 7,
                sequence: 0
            })
        );
        assert_eq!(pipeline.try_take().unwrap().id().sequence, 1);
        assert_eq!(pipeline.try_take().unwrap().id().sequence, 2);
        assert!(pipeline.try_take().is_none());

        let snapshot = pipeline.snapshot();
        assert_eq!(snapshot.stats.accepted, 3);
        assert_eq!(snapshot.stats.dropped_capacity, 1);
        assert_eq!(snapshot.stats.dequeued, 2);
    }

    #[test]
    fn generation_transition_clears_and_rejects_late_frames() {
        let pipeline = FramePipeline::new(2, 3).unwrap();
        assert_eq!(pipeline.submit(frame(3, 4)), SubmitOutcome::Queued);
        assert_eq!(pipeline.begin_generation(4).unwrap(), 1);
        assert_eq!(
            pipeline.submit(frame(3, 5)),
            SubmitOutcome::RejectedGeneration {
                active: 4,
                received: 3
            }
        );
        assert_eq!(pipeline.submit(frame(4, 0)), SubmitOutcome::Queued);
        assert_eq!(
            pipeline.submit(frame(4, 0)),
            SubmitOutcome::RejectedSequence {
                last: 0,
                received: 0
            }
        );
        assert!(matches!(
            pipeline.begin_generation(4),
            Err(PipelineError::GenerationNotIncreasing { .. })
        ));

        let snapshot = pipeline.snapshot();
        assert_eq!(snapshot.stats.discarded_generation, 1);
        assert_eq!(snapshot.stats.rejected_generation, 1);
        assert_eq!(snapshot.stats.rejected_sequence, 1);
    }

    #[test]
    fn sink_gate_never_runs_for_an_old_generation() {
        let pipeline = FramePipeline::new(1, 9).unwrap();
        let old_id = FrameId {
            generation: 9,
            sequence: 3,
        };
        pipeline.begin_generation(10).unwrap();

        let mut called = false;
        assert_eq!(
            pipeline.with_current_generation(old_id, || called = true),
            Err(GenerationMismatch {
                active: 10,
                received: 9
            })
        );
        assert!(!called);
        assert_eq!(pipeline.snapshot().stats.rejected_sink_generation, 1);

        let current_id = FrameId {
            generation: 10,
            sequence: 0,
        };
        assert_eq!(pipeline.with_current_generation(current_id, || 42), Ok(42));
    }

    #[test]
    fn generation_transition_waits_for_atomic_sink_submission() {
        use std::sync::{Arc, mpsc};
        use std::thread;

        let pipeline = Arc::new(FramePipeline::new(1, 2).unwrap());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let sink_pipeline = Arc::clone(&pipeline);
        let sink = thread::spawn(move || {
            sink_pipeline
                .with_current_generation(
                    FrameId {
                        generation: 2,
                        sequence: 0,
                    },
                    || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    },
                )
                .unwrap();
        });
        entered_rx.recv().unwrap();

        let (advanced_tx, advanced_rx) = mpsc::channel();
        let transition_pipeline = Arc::clone(&pipeline);
        let transition = thread::spawn(move || {
            transition_pipeline.begin_generation(3).unwrap();
            advanced_tx.send(()).unwrap();
        });
        assert!(advanced_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        sink.join().unwrap();
        transition.join().unwrap();
        advanced_rx.recv().unwrap();
        assert_eq!(pipeline.snapshot().generation, 3);
    }

    #[test]
    fn pipeline_and_frames_are_send_and_sync_as_intended() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send::<Frame>();
        assert_send_sync::<FramePipeline>();
    }
}
