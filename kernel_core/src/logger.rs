//! A lock-free concurrent logger.
use core::cell::UnsafeCell;
use core::fmt::Write;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicUsize, Ordering};
use log::{Level, LevelFilter, Log, Metadata, Record};
use spin::Mutex;

/// Returns the ANSI color code for a given log level.
fn color_for_level(lvl: Level) -> &'static str {
    match lvl {
        Level::Error => "31",
        Level::Warn => "33",
        Level::Info => "32",
        Level::Debug => "34",
        Level::Trace => "35",
    }
}

/// Trait for reading global values like core ID and timer counter.
pub trait GlobalValueReader {
    fn read() -> GlobalValues;
}

/// Struct to hold global values like core ID and timer counter.
pub struct GlobalValues {
    pub core_id: usize,
    pub timer_counter: u64,
}

/// Trait representing a sink that accepts log chunks.
pub trait LogSink {
    /// Accepts a log chunk.
    fn accept(&mut self, chunk: &[u8]);
}

const MAX_LOG_CHUNK_SIZE: usize = 120;

/// A guard that provides safe access to the chunk's buffer during writing
pub struct ChunkWriteGuard<'a> {
    chunk: &'a LogChunk,
    buffer: &'a mut [u8],
}

impl ChunkWriteGuard<'_> {
    /// Marks the chunk as full with the given size and consumes the guard
    pub fn finish(self, actual_size: usize) {
        // Ensure that data writes are visible before updating the status
        let status_and_size = STATUS_FULL | (actual_size << SIZE_SHIFT);
        self.chunk
            .status_and_size
            .store(status_and_size, Ordering::Release);
    }
}

impl core::ops::Deref for ChunkWriteGuard<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.buffer
    }
}

impl core::ops::DerefMut for ChunkWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer
    }
}

/// Represents a chunk of log data.
struct LogChunk {
    data: UnsafeCell<[u8; MAX_LOG_CHUNK_SIZE]>,
    status_and_size: AtomicUsize,
}

unsafe impl Sync for LogChunk {}

impl LogChunk {
    /// Creates a new, empty `LogChunk`.
    const fn new() -> Self {
        Self {
            data: UnsafeCell::new([0; MAX_LOG_CHUNK_SIZE]),
            status_and_size: AtomicUsize::new(STATUS_EMPTY),
        }
    }

    /// Attempts to acquire the chunk for writing.
    /// Returns a [`ChunkWriteGuard`] that provides safe access to the buffer and must be used
    /// to mark the chunk as full when writing is complete.
    fn try_acquire_for_write(&self) -> Option<ChunkWriteGuard<'_>> {
        self.status_and_size
            .compare_exchange(
                STATUS_EMPTY,
                STATUS_WRITING,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .ok()
            .and_then(|_| unsafe {
                // SAFETY: we use the atomic to make sure that we have exclusive access to the data
                // buffer before creating the WriteGuard, and we will have that exclusive access until
                // finish is called, which consumes the guard.
                self.data.get().as_mut()
            })
            .map(|a| ChunkWriteGuard {
                chunk: self,
                buffer: &mut a[..],
            })
    }

    /// Attempts to read data from the chunk and processes it using the provided closure.
    fn try_read<F>(&self, f: F) -> bool
    where
        F: FnOnce(&[u8]),
    {
        let status_and_size = self.status_and_size.load(Ordering::Acquire);
        if (status_and_size & STATUS_MASK) == STATUS_FULL {
            let size = (status_and_size & SIZE_MASK) >> SIZE_SHIFT;
            let data = unsafe {
                // SAFETY: we know no one is mutating because the chunk is [`STATUS_FULL`].
                // It is possible two reads could happen concurrently, but this is probably fine.
                &*self.data.get()
            };
            let data = &data[..size];
            // Process the data while the chunk is still marked as full.
            f(data);
            // Mark the chunk as empty after processing.
            self.status_and_size.store(STATUS_EMPTY, Ordering::Release);
            true
        } else {
            false
        }
    }
}

const STATUS_EMPTY: usize = 0;
const STATUS_WRITING: usize = 1;
const STATUS_FULL: usize = 2;
const STATUS_MASK: usize = 0b11; // Lower 2 bits.
const SIZE_SHIFT: usize = 2;
const SIZE_MASK: usize = !STATUS_MASK;

/// A lock-free concurrent logger with a ring buffer.
///
/// By default `NUM_CHUNKS_IN_BUFFER = 128`, which is a 16KiB buffer.
pub struct Logger<S, G, const NUM_CHUNKS_IN_BUFFER: usize = 128> {
    _global_value_reader: PhantomData<G>,
    buffer: [LogChunk; NUM_CHUNKS_IN_BUFFER],
    write_index: AtomicUsize,
    read_index: AtomicUsize,
    overflow_count: AtomicUsize,
    sink: Mutex<S>,
    level_filter: LevelFilter,
}

impl<S: LogSink, G: GlobalValueReader, const NUM_CHUNKS_IN_BUFFER: usize> Logger<S, G, NUM_CHUNKS_IN_BUFFER> {
    /// Creates a new `Logger` with the specified sink and log level filter.
    pub fn new(sink: S, level_filter: LevelFilter) -> Self {
        Self {
            buffer: [const { LogChunk::new() }; NUM_CHUNKS_IN_BUFFER],
            write_index: AtomicUsize::new(0),
            read_index: AtomicUsize::new(0),
            overflow_count: AtomicUsize::new(0),
            sink: Mutex::new(sink),
            level_filter,
            _global_value_reader: PhantomData,
        }
    }

    /// Write a record into the buffer.
    fn write_record(&self, record: &Record) {
        // Create a RingBufferWriter.
        let mut writer = RingBufferWriter::new(self);

        let module_path = record.module_path().unwrap_or("unknown module");
        let line = record.line().unwrap_or(0);

        // Read global values.
        let global_values = G::read();

        // Write formatted data directly into the ring buffer.
        writeln!(
            &mut writer,
            "\x1b[{}m{:<5}\x1b[0m {}@{}| Core: {} Timer: {} | {}",
            color_for_level(record.level()),
            record.level(),
            module_path,
            line,
            global_values.core_id,
            global_values.timer_counter,
            record.args()
        )
        .unwrap();
    }

    /// Flush up to `limit` log chunks to the sink, given that we could acquire it.
    fn flush_internal(&self, sink: &mut S, limit: usize) {
        // Send overflow message if any logs were lost.
        let overflow_count = self.overflow_count.swap(0, Ordering::Acquire);
        if overflow_count > 0 {
            sink.accept(b"\x1b[31mlog overflow!\x1b[0m");
        }

        for _ in 0..limit {
            let read_index = self.read_index.load(Ordering::Acquire);
            let write_index = self.write_index.load(Ordering::Acquire);

            if read_index == write_index {
                // No more chunks to read.
                break;
            }

            let wrapped_index = read_index % NUM_CHUNKS_IN_BUFFER;
            let chunk = &self.buffer[wrapped_index];

            // Safely read and process the chunk.
            let read_success = chunk.try_read(|data| {
                sink.accept(data);
            });

            if read_success {
                self.read_index.fetch_add(1, Ordering::Release);
            } else {
                // Chunk not ready; avoid busy waiting.
                break;
            }
        }
    }

    /// Write a record into the buffer.
    fn write_record(&self, record: &Record) {
        // Create a RingBufferWriter.
        let mut writer = RingBufferWriter::new(self);

        let module_path = record.module_path().unwrap_or("unknown module");
        let line = record.line().unwrap_or(0);

        // Write formatted data directly into the ring buffer.
        writeln!(
            &mut writer,
            "\x1b[{}m{:<5}\x1b[0m {}@{}| {}",
            color_for_level(record.level()),
            record.level(),
            module_path,
            line,
            record.args()
        )
        .unwrap();
    }
}

impl<S: LogSink + Send, G: GlobalValueReader, const NUM_CHUNKS_IN_BUFFER: usize> Log for Logger<S, G, NUM_CHUNKS_IN_BUFFER> {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level_filter
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        self.write_record(record);

        // Attempt to flush the buffer if possible.
        if let Some(mut sink_guard) = self.sink.try_lock() {
            self.flush_internal(&mut *sink_guard, NUM_CHUNKS_IN_BUFFER / 3);
        }
    }

    fn flush(&self) {
        let mut sink_guard = self.sink.lock();
        self.flush_internal(&mut *sink_guard, NUM_CHUNKS_IN_BUFFER);
    }
}

/// A writer that writes directly into the ring buffer.
struct RingBufferWriter<'a, S: LogSink, const N: usize> {
    logger: &'a Logger<S, N>,
    current_chunk: Option<ChunkWriteGuard<'a>>,
    current_chunk_offset: usize,
}

impl<'a, S: LogSink, const N: usize> RingBufferWriter<'a, S, N> {
    /// Creates a new `RingBufferWriter`.
    fn new(logger: &'a Logger<S, N>) -> Self {
        Self {
            logger,
            current_chunk: None,
            current_chunk_offset: 0,
        }
    }

    /// Acquires a new chunk to write to.
    fn acquire_new_chunk(&mut self) -> Result<(), ()> {
        // Get the next index to write to.
        let index = self.logger.write_index.fetch_add(1, Ordering::AcqRel);

        // Calculate the distance to detect overflows.
        let read_index = self.logger.read_index.load(Ordering::Acquire);
        let distance = index.wrapping_sub(read_index);

        if distance >= N {
            // Buffer overflow occurred; increment overflow count and return error.
            self.logger.overflow_count.fetch_add(1, Ordering::Relaxed);
            return Err(());
        }

        let wrapped_index = index % N;
        let chunk = &self.logger.buffer[wrapped_index];

        // Try to acquire the chunk for writing.
        if let Some(wg) = chunk.try_acquire_for_write() {
            self.current_chunk = Some(wg);
            self.current_chunk_offset = 0;
            Ok(())
        } else {
            Err(())
        }
    }

    /// Finish the chunk we're currently writing in.
    fn finish_chunk(&mut self) {
        if let Some(chunk) = self.current_chunk.take() {
            chunk.finish(self.current_chunk_offset);
        }
    }
}

impl<S: LogSink, const N: usize> core::fmt::Write for RingBufferWriter<'_, S, N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut s = s.as_bytes();
        while !s.is_empty() {
            // If no current chunk or current chunk is full, acquire a new one.
            if self.current_chunk.is_none() || self.current_chunk_offset >= MAX_LOG_CHUNK_SIZE {
                // Mark the previous chunk as full.
                self.finish_chunk();

                // Try to acquire a new chunk.
                if self.acquire_new_chunk().is_err() {
                    // Increment overflow count and discard the remaining data.
                    self.logger.overflow_count.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
            }

            let chunk = self.current_chunk.as_mut().unwrap();
            let remaining_space = MAX_LOG_CHUNK_SIZE - self.current_chunk_offset;
            let bytes_to_copy = core::cmp::min(remaining_space, s.len());

            // Copy the data into the chunk.
            let dest =
                &mut chunk[self.current_chunk_offset..self.current_chunk_offset + bytes_to_copy];
            dest.copy_from_slice(&s[..bytes_to_copy]);

            self.current_chunk_offset += bytes_to_copy;
            s = &s[bytes_to_copy..];
        }

        Ok(())
    }
}

impl<S: LogSink, const N: usize> Drop for RingBufferWriter<'_, S, N> {
    fn drop(&mut self) {
        self.finish_chunk();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{string::String, sync::Arc, thread, time::Duration, vec::Vec};

    // Test sink that collects log messages
    #[derive(Default)]
    struct TestSink {
        messages: Vec<Vec<u8>>,
    }

    impl LogSink for TestSink {
        fn accept(&mut self, chunk: &[u8]) {
            self.messages.push(chunk.to_vec());
        }
    }

    impl TestSink {
        fn get_messages_as_string(&self) -> Vec<String> {
            self.messages
                .iter()
                .map(|msg| String::from_utf8_lossy(msg).into_owned())
                .collect()
        }
    }

    #[test]
    fn test_basic_logging() {
        let logger = Logger::<TestSink, 16>::new(TestSink::default(), LevelFilter::Info);

        let record = Record::builder()
            .args(format_args!("test message"))
            .level(Level::Info)
            .target("test")
            .module_path(Some("test_module"))
            .file(Some("test.rs"))
            .line(Some(42))
            .build();

        logger.log(&record);

        let messages = logger.sink.lock().get_messages_as_string();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("test message"));
        assert!(messages[0].contains("test_module@42"));
    }

    #[test]
    fn test_log_level_filtering() {
        let logger = Logger::<TestSink, 16>::new(TestSink::default(), LevelFilter::Warn);

        // This should be logged
        let warn_record = Record::builder()
            .args(format_args!("warning"))
            .level(Level::Warn)
            .target("test")
            .build();

        // This should be filtered out
        let info_record = Record::builder()
            .args(format_args!("info"))
            .level(Level::Info)
            .target("test")
            .build();

        logger.log(&warn_record);
        logger.log(&info_record);
        logger.flush();

        let messages = logger.sink.lock().get_messages_as_string();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("warning"));
    }

    #[test]
    fn test_buffer_overflow() {
        // Use a very small buffer to test overflow
        let logger = Logger::<TestSink, 2>::new(TestSink::default(), LevelFilter::Info);

        // lock the sink to prevent a flush
        let sink = logger.sink.lock();

        // Fill the buffer
        for _ in 0..5 {
            let mut record = Record::builder();
            record
                .args(format_args!("message"))
                .level(Level::Info)
                .target("test");
            logger.log(&record.build());

            // Small delay to ensure messages are processed in order
            thread::sleep(Duration::from_millis(1));
        }

        // allow a flush to occur again
        drop(sink);
        logger.flush();

        let messages = logger.sink.lock().get_messages_as_string();

        // Check for overflow message
        assert!(messages.iter().any(|msg| msg.contains("overflow")));
    }

    #[test]
    fn test_concurrent_logging() {
        let logger = Arc::new(Logger::<TestSink, 32>::new(
            TestSink::default(),
            LevelFilter::Info,
        ));
        let thread_count = 8;
        let messages_per_thread = 100;

        thread::scope(|scope| {
            let mut handles = Vec::new();

            for thread_id in 0..thread_count {
                let logger = Arc::clone(&logger);
                handles.push(scope.spawn(move || {
                    for msg_id in 0..messages_per_thread {
                        logger.log(
                            &Record::builder()
                                .args(format_args!("Thread {thread_id} Message {msg_id}"))
                                .level(Level::Info)
                                .target("test")
                                .build(),
                        );
                    }
                }));
            }

            // Wait for all threads to complete
            for handle in handles {
                handle.join().unwrap();
            }
        });

        logger.flush();
        let messages = logger.sink.lock().get_messages_as_string();

        // Count actual messages (excluding overflow messages)
        let actual_messages: Vec<_> = messages
            .iter()
            .filter(|msg| !msg.contains("overflow"))
            .collect();

        // We might have some messages lost due to overflow, but we should have some messages
        assert!(!actual_messages.is_empty());

        // Verify message integrity
        for msg in actual_messages {
            assert!(msg.contains("Thread") && msg.contains("Message"));
        }
    }

    #[test]
    fn test_large_message_chunking() {
        let logger = Logger::<TestSink, 16>::new(TestSink::default(), LevelFilter::Info);

        // Create a message larger than MAX_LOG_CHUNK_SIZE
        let large_message = "A".repeat(MAX_LOG_CHUNK_SIZE * 2);

        logger.log(
            &Record::builder()
                .args(format_args!("{large_message}"))
                .level(Level::Info)
                .target("test")
                .build(),
        );
        logger.flush();

        let messages = logger.sink.lock().get_messages_as_string();

        // Message should be split into chunks
        assert!(messages.len() > 1);

        // Concatenate all chunks and verify content
        let full_message: String = messages
            .iter()
            .map(|msg| msg.replace(&['\x1b', '[', '0', 'm', '3', '2'][..], ""))
            .collect();

        assert!(full_message.contains(&large_message));
    }

    #[test]
    fn test_empty_message() {
        let logger = Logger::<TestSink, 16>::new(TestSink::default(), LevelFilter::Info);

        let record = Record::builder()
            .args(format_args!(""))
            .level(Level::Info)
            .target("test")
            .build();

        logger.log(&record);
        logger.flush();

        let messages = logger.sink.lock().get_messages_as_string();
        assert!(!messages.is_empty()); // Should still log the metadata
    }

    #[test]
    fn test_concurrent_flush() {
        let logger = Arc::new(Logger::<TestSink, 16>::new(
            TestSink::default(),
            LevelFilter::Info,
        ));
        let flush_count = 100;

        thread::scope(|scope| {
            let mut handles = Vec::new();

            // Multiple threads trying to flush simultaneously
            for _ in 0..4 {
                let logger = Arc::clone(&logger);
                handles.push(scope.spawn(move || {
                    for _ in 0..flush_count {
                        logger.flush();
                    }
                }));
            }

            // One thread continuously logging while others flush
            let logger2 = logger.clone();
            handles.push(scope.spawn(move || {
                for i in 0..flush_count {
                    logger2.log(
                        &Record::builder()
                            .args(format_args!("Message {i}"))
                            .level(Level::Info)
                            .target("test")
                            .build(),
                    );
                    thread::sleep(Duration::from_micros(10));
                }
            }));

            for handle in handles {
                handle.join().unwrap();
            }
        });

        // Final flush to ensure all messages are processed
        logger.flush();

        let messages = logger.sink.lock().get_messages_as_string();
        assert!(!messages.is_empty());
    }

    #[test]
    fn test_wrapped_indices() {
        let logger = Logger::<TestSink, 4>::new(TestSink::default(), LevelFilter::Info);

        // Force the write_index to wrap around
        logger.write_index.store(usize::MAX - 2, Ordering::SeqCst);
        logger.read_index.store(usize::MAX - 2, Ordering::SeqCst);

        for i in 0..8 {
            logger.log(
                &Record::builder()
                    .args(format_args!("Wrap message {i}"))
                    .level(Level::Info)
                    .target("test")
                    .build(),
            );
        }

        logger.flush();
        let messages = logger.sink.lock().get_messages_as_string();

        // Should have some messages and potentially an overflow message
        assert!(!messages.is_empty());
        assert!(messages.iter().any(|msg| msg.contains("Wrap message")));
    }

    #[test]
    fn test_all_log_levels() {
        let logger = Logger::<TestSink, 16>::new(TestSink::default(), LevelFilter::Trace);

        for level in &[
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            logger.log(
                &Record::builder()
                    .args(format_args!("{level} message"))
                    .level(*level)
                    .target("test")
                    .build(),
            );
        }

        logger.flush();
        let messages = logger.sink.lock().get_messages_as_string();

        assert_eq!(messages.len(), 5);
        assert!(messages.iter().any(|msg| msg.contains("ERROR")));
        assert!(messages.iter().any(|msg| msg.contains("WARN")));
        assert!(messages.iter().any(|msg| msg.contains("INFO")));
        assert!(messages.iter().any(|msg| msg.contains("DEBUG")));
        assert!(messages.iter().any(|msg| msg.contains("TRACE")));
    }
}
