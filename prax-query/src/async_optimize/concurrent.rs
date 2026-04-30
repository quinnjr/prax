//! Concurrent task execution with controlled parallelism.
//!
//! This module provides utilities for executing multiple independent database
//! operations in parallel while respecting concurrency limits to avoid
//! overwhelming the database connection pool.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

/// Configuration for concurrent execution.
#[derive(Debug, Clone)]
pub struct ConcurrencyConfig {
    /// Maximum number of concurrent operations.
    pub max_concurrency: usize,
    /// Timeout for individual operations.
    pub operation_timeout: Option<Duration>,
    /// Whether to continue on error (collect all errors vs fail fast).
    pub continue_on_error: bool,
    /// Collect timing statistics.
    pub collect_stats: bool,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            max_concurrency: num_cpus::get().max(4),
            operation_timeout: Some(Duration::from_secs(30)),
            continue_on_error: true,
            collect_stats: true,
        }
    }
}

impl ConcurrencyConfig {
    /// Create config optimized for database introspection.
    #[must_use]
    pub fn for_introspection() -> Self {
        Self {
            max_concurrency: 8, // Balance between speed and connection usage
            operation_timeout: Some(Duration::from_secs(60)),
            continue_on_error: true,
            collect_stats: true,
        }
    }

    /// Create config optimized for migration operations.
    #[must_use]
    pub fn for_migrations() -> Self {
        Self {
            max_concurrency: 4, // More conservative for DDL
            operation_timeout: Some(Duration::from_secs(120)),
            continue_on_error: false, // Migrations should fail fast
            collect_stats: true,
        }
    }

    /// Create config optimized for bulk data operations.
    #[must_use]
    pub fn for_bulk_operations() -> Self {
        Self {
            max_concurrency: 16, // Higher parallelism for DML
            operation_timeout: Some(Duration::from_secs(300)),
            continue_on_error: true,
            collect_stats: true,
        }
    }

    /// Set maximum concurrency.
    #[must_use]
    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    /// Set operation timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = Some(timeout);
        self
    }

    /// Disable timeout.
    #[must_use]
    pub fn without_timeout(mut self) -> Self {
        self.operation_timeout = None;
        self
    }

    /// Set continue on error behavior.
    #[must_use]
    pub fn with_continue_on_error(mut self, continue_on_error: bool) -> Self {
        self.continue_on_error = continue_on_error;
        self
    }
}

/// Error from concurrent task execution.
#[derive(Debug, Clone)]
pub struct TaskError {
    /// Task identifier.
    pub task_id: usize,
    /// Error message.
    pub message: String,
    /// Whether this was a timeout.
    pub is_timeout: bool,
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_timeout {
            write!(f, "Task {} timed out: {}", self.task_id, self.message)
        } else {
            write!(f, "Task {} failed: {}", self.task_id, self.message)
        }
    }
}

impl std::error::Error for TaskError {}

/// Result of a single task.
#[derive(Debug)]
pub enum TaskResult<T> {
    /// Task completed successfully.
    Success {
        /// Task identifier.
        task_id: usize,
        /// The result value.
        value: T,
        /// Execution duration.
        duration: Duration,
    },
    /// Task failed.
    Error(TaskError),
}

impl<T> TaskResult<T> {
    /// Check if the task succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// Get the value if successful.
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Success { value, .. } => Some(value),
            Self::Error(_) => None,
        }
    }

    /// Get the error if failed.
    pub fn into_error(self) -> Option<TaskError> {
        match self {
            Self::Success { .. } => None,
            Self::Error(e) => Some(e),
        }
    }
}

/// Statistics from concurrent execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    /// Total tasks processed.
    pub total_tasks: u64,
    /// Successful tasks.
    pub successful: u64,
    /// Failed tasks.
    pub failed: u64,
    /// Timed out tasks.
    pub timed_out: u64,
    /// Total execution time.
    pub total_duration: Duration,
    /// Average task duration (for successful tasks).
    pub avg_task_duration: Duration,
    /// Maximum concurrent tasks observed.
    pub max_concurrent: usize,
}

/// Executor for running concurrent tasks with controlled parallelism.
pub struct ConcurrentExecutor {
    config: ConcurrencyConfig,
    semaphore: Arc<Semaphore>,
    stats: ExecutionStatsCollector,
}

impl ConcurrentExecutor {
    /// Create a new concurrent executor.
    pub fn new(config: ConcurrencyConfig) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_concurrency));
        Self {
            config,
            semaphore,
            stats: ExecutionStatsCollector::new(),
        }
    }

    /// Execute all tasks concurrently with controlled parallelism.
    ///
    /// Tasks are started immediately but limited by the semaphore to ensure
    /// at most `max_concurrency` tasks run at once.
    pub async fn execute_all<T, F, Fut>(
        &self,
        tasks: impl IntoIterator<Item = F>,
    ) -> (Vec<TaskResult<T>>, ExecutionStats)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
    {
        let start = Instant::now();
        self.stats.reset();

        let tasks: Vec<_> = tasks.into_iter().collect();
        let total_tasks = tasks.len();
        self.stats.total.store(total_tasks as u64, Ordering::SeqCst);

        let mut futures = FuturesUnordered::new();

        for (task_id, task) in tasks.into_iter().enumerate() {
            let semaphore = Arc::clone(&self.semaphore);
            let timeout = self.config.operation_timeout;
            let stats = self.stats.clone();

            let future = async move {
                // Acquire semaphore permit
                let _permit = semaphore.acquire().await.expect("Semaphore closed");
                stats.increment_concurrent();

                let task_start = Instant::now();
                let result = if let Some(timeout_duration) = timeout {
                    match tokio::time::timeout(timeout_duration, task()).await {
                        Ok(Ok(value)) => TaskResult::Success {
                            task_id,
                            value,
                            duration: task_start.elapsed(),
                        },
                        Ok(Err(msg)) => TaskResult::Error(TaskError {
                            task_id,
                            message: msg,
                            is_timeout: false,
                        }),
                        Err(_) => TaskResult::Error(TaskError {
                            task_id,
                            message: format!("Timeout after {:?}", timeout_duration),
                            is_timeout: true,
                        }),
                    }
                } else {
                    match task().await {
                        Ok(value) => TaskResult::Success {
                            task_id,
                            value,
                            duration: task_start.elapsed(),
                        },
                        Err(msg) => TaskResult::Error(TaskError {
                            task_id,
                            message: msg,
                            is_timeout: false,
                        }),
                    }
                };

                stats.decrement_concurrent();

                match &result {
                    TaskResult::Success { duration, .. } => {
                        stats.record_success(*duration);
                    }
                    TaskResult::Error(e) if e.is_timeout => {
                        stats.record_timeout();
                    }
                    TaskResult::Error(_) => {
                        stats.record_failure();
                    }
                }

                result
            };

            futures.push(future);
        }

        // Collect results in order of completion
        let mut results = Vec::with_capacity(total_tasks);

        while let Some(result) = futures.next().await {
            if !self.config.continue_on_error
                && let TaskResult::Error(ref _e) = result
            {
                // Cancel remaining futures by dropping them
                drop(futures);
                results.push(result);

                let stats = self.stats.finalize(start.elapsed());
                return (results, stats);
            }
            results.push(result);
        }

        // Sort by task_id to maintain original order
        results.sort_by_key(|r| match r {
            TaskResult::Success { task_id, .. } => *task_id,
            TaskResult::Error(e) => e.task_id,
        });

        let stats = self.stats.finalize(start.elapsed());
        (results, stats)
    }

    /// Execute tasks and collect only successful results.
    ///
    /// Returns the values in the same order as the input tasks.
    pub async fn execute_collect<T, F, Fut>(
        &self,
        tasks: impl IntoIterator<Item = F>,
    ) -> (Vec<T>, Vec<TaskError>)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
    {
        let (results, _) = self.execute_all(tasks).await;

        let mut values = Vec::new();
        let mut errors = Vec::new();

        for result in results {
            match result {
                TaskResult::Success { value, .. } => values.push(value),
                TaskResult::Error(e) => errors.push(e),
            }
        }

        (values, errors)
    }

    /// Execute tasks with indexed results.
    ///
    /// Returns a map of task_id -> result, useful when you need to correlate
    /// results with their original indices.
    pub async fn execute_indexed<T, F, Fut>(
        &self,
        tasks: impl IntoIterator<Item = F>,
    ) -> std::collections::HashMap<usize, Result<T, TaskError>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
    {
        let (results, _) = self.execute_all(tasks).await;

        results
            .into_iter()
            .map(|r| match r {
                TaskResult::Success { task_id, value, .. } => (task_id, Ok(value)),
                TaskResult::Error(e) => (e.task_id, Err(e)),
            })
            .collect()
    }
}

/// Internal stats collector with atomic counters.
#[derive(Clone)]
struct ExecutionStatsCollector {
    total: Arc<AtomicU64>,
    successful: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    timed_out: Arc<AtomicU64>,
    total_task_duration_ns: Arc<AtomicU64>,
    current_concurrent: Arc<AtomicU64>,
    max_concurrent: Arc<AtomicU64>,
}

impl ExecutionStatsCollector {
    fn new() -> Self {
        Self {
            total: Arc::new(AtomicU64::new(0)),
            successful: Arc::new(AtomicU64::new(0)),
            failed: Arc::new(AtomicU64::new(0)),
            timed_out: Arc::new(AtomicU64::new(0)),
            total_task_duration_ns: Arc::new(AtomicU64::new(0)),
            current_concurrent: Arc::new(AtomicU64::new(0)),
            max_concurrent: Arc::new(AtomicU64::new(0)),
        }
    }

    fn reset(&self) {
        self.total.store(0, Ordering::SeqCst);
        self.successful.store(0, Ordering::SeqCst);
        self.failed.store(0, Ordering::SeqCst);
        self.timed_out.store(0, Ordering::SeqCst);
        self.total_task_duration_ns.store(0, Ordering::SeqCst);
        self.current_concurrent.store(0, Ordering::SeqCst);
        self.max_concurrent.store(0, Ordering::SeqCst);
    }

    fn increment_concurrent(&self) {
        let current = self.current_concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_concurrent.fetch_max(current, Ordering::SeqCst);
    }

    fn decrement_concurrent(&self) {
        self.current_concurrent.fetch_sub(1, Ordering::SeqCst);
    }

    fn record_success(&self, duration: Duration) {
        self.successful.fetch_add(1, Ordering::SeqCst);
        self.total_task_duration_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::SeqCst);
    }

    fn record_failure(&self) {
        self.failed.fetch_add(1, Ordering::SeqCst);
    }

    fn record_timeout(&self) {
        self.timed_out.fetch_add(1, Ordering::SeqCst);
        self.failed.fetch_add(1, Ordering::SeqCst);
    }

    fn finalize(&self, total_duration: Duration) -> ExecutionStats {
        let successful = self.successful.load(Ordering::SeqCst);
        let total_task_duration_ns = self.total_task_duration_ns.load(Ordering::SeqCst);

        let avg_task_duration = total_task_duration_ns
            .checked_div(successful)
            .map(Duration::from_nanos)
            .unwrap_or(Duration::ZERO);

        ExecutionStats {
            total_tasks: self.total.load(Ordering::SeqCst),
            successful,
            failed: self.failed.load(Ordering::SeqCst),
            timed_out: self.timed_out.load(Ordering::SeqCst),
            total_duration,
            avg_task_duration,
            max_concurrent: self.max_concurrent.load(Ordering::SeqCst) as usize,
        }
    }
}

/// Helper for executing a batch of similar operations concurrently.
///
/// This is a convenience function for common patterns like fetching
/// metadata for multiple tables.
pub async fn execute_batch<T, I, F, Fut>(
    items: I,
    max_concurrency: usize,
    operation: F,
) -> Vec<Result<T, String>>
where
    I: IntoIterator,
    F: Fn(I::Item) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
    I::Item: Send + 'static,
{
    let config = ConcurrencyConfig::default().with_max_concurrency(max_concurrency);
    let executor = ConcurrentExecutor::new(config);

    let tasks: Vec<_> = items
        .into_iter()
        .map(|item| {
            let op = operation.clone();
            move || op(item)
        })
        .collect();

    let (results, _) = executor.execute_all(tasks).await;

    results
        .into_iter()
        .map(|r| match r {
            TaskResult::Success { value, .. } => Ok(value),
            TaskResult::Error(e) => Err(e.message),
        })
        .collect()
}

/// Execute operations in parallel chunks.
///
/// Useful for operations that benefit from batching (like multi-row inserts)
/// combined with parallel execution of batches.
pub async fn execute_chunked<T, I, F, Fut>(
    items: I,
    chunk_size: usize,
    max_concurrency: usize,
    operation: F,
) -> Vec<Vec<Result<T, String>>>
where
    I: IntoIterator,
    I::IntoIter: ExactSizeIterator,
    F: Fn(Vec<I::Item>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Vec<Result<T, String>>> + Send + 'static,
    T: Send + 'static,
    I::Item: Send + Clone + 'static,
{
    let items: Vec<_> = items.into_iter().collect();
    let chunks: Vec<Vec<_>> = items.chunks(chunk_size).map(|c| c.to_vec()).collect();

    let config = ConcurrencyConfig::default().with_max_concurrency(max_concurrency);
    let executor = ConcurrentExecutor::new(config);

    let tasks: Vec<_> = chunks
        .into_iter()
        .map(|chunk| {
            let op = operation.clone();
            move || async move { Ok::<_, String>(op(chunk).await) }
        })
        .collect();

    let (results, _) = executor.execute_all(tasks).await;

    results.into_iter().filter_map(|r| r.into_value()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn test_concurrent_executor_basic() {
        let executor = ConcurrentExecutor::new(ConcurrencyConfig::default());

        let tasks: Vec<_> = (0..10)
            .map(|i| move || async move { Ok::<_, String>(i * 2) })
            .collect();

        let (results, stats) = executor.execute_all(tasks).await;

        assert_eq!(results.len(), 10);
        assert_eq!(stats.total_tasks, 10);
        assert_eq!(stats.successful, 10);
        assert_eq!(stats.failed, 0);

        // Verify results are in order
        for (i, result) in results.iter().enumerate() {
            match result {
                TaskResult::Success { value, .. } => {
                    assert_eq!(*value, i * 2);
                }
                _ => panic!("Expected success"),
            }
        }
    }

    #[tokio::test]
    async fn test_concurrent_executor_with_errors() {
        let config = ConcurrencyConfig::default().with_continue_on_error(true);
        let executor = ConcurrentExecutor::new(config);

        let tasks: Vec<_> = (0..5)
            .map(|i| {
                move || async move {
                    if i == 2 {
                        Err("Task 2 failed".to_string())
                    } else {
                        Ok::<_, String>(i)
                    }
                }
            })
            .collect();

        let (results, stats) = executor.execute_all(tasks).await;

        assert_eq!(results.len(), 5);
        assert_eq!(stats.successful, 4);
        assert_eq!(stats.failed, 1);
    }

    #[tokio::test]
    async fn test_concurrent_executor_fail_fast() {
        let config = ConcurrencyConfig::default()
            .with_continue_on_error(false)
            .with_max_concurrency(1); // Sequential to ensure order

        let executor = ConcurrentExecutor::new(config);
        let counter = Arc::new(AtomicUsize::new(0));

        let tasks: Vec<_> = (0..5)
            .map(|i| {
                let counter = Arc::clone(&counter);
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    if i == 2 {
                        Err("Task 2 failed".to_string())
                    } else {
                        Ok::<_, String>(i)
                    }
                }
            })
            .collect();

        let (results, _) = executor.execute_all(tasks).await;

        // Should have stopped at first error - check using pattern match
        let has_error = results.iter().any(|r| matches!(r, TaskResult::Error(_)));
        assert!(has_error);
    }

    #[tokio::test]
    async fn test_concurrent_executor_respects_concurrency() {
        let max_concurrent = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));

        let config = ConcurrencyConfig::default().with_max_concurrency(3);
        let executor = ConcurrentExecutor::new(config);

        let tasks: Vec<_> = (0..20)
            .map(|i| {
                let max_concurrent = Arc::clone(&max_concurrent);
                let current = Arc::clone(&current);
                move || async move {
                    let c = current.fetch_add(1, Ordering::SeqCst) + 1;
                    max_concurrent.fetch_max(c, Ordering::SeqCst);

                    // Simulate work
                    tokio::time::sleep(Duration::from_millis(10)).await;

                    current.fetch_sub(1, Ordering::SeqCst);
                    Ok::<_, String>(i)
                }
            })
            .collect();

        let (results, stats) = executor.execute_all(tasks).await;

        assert_eq!(results.len(), 20);
        assert!(stats.max_concurrent <= 3);
        assert!(max_concurrent.load(Ordering::SeqCst) <= 3);
    }

    #[tokio::test]
    async fn test_execute_batch() {
        let results = execute_batch(vec!["a", "b", "c"], 4, |s: &str| async move {
            Ok::<_, String>(s.len())
        })
        .await;

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[tokio::test]
    async fn test_timeout() {
        let config = ConcurrencyConfig::default().with_timeout(Duration::from_millis(50));
        let executor = ConcurrentExecutor::new(config);

        #[allow(clippy::type_complexity)]
        let tasks: Vec<
            Box<
                dyn FnOnce() -> std::pin::Pin<Box<dyn Future<Output = Result<i32, String>> + Send>>
                    + Send,
            >,
        > = vec![
            Box::new(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Ok::<_, String>(1)
                })
            }),
            Box::new(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok::<_, String>(2)
                })
            }),
        ];

        let (results, stats) = executor.execute_all(tasks).await;

        assert_eq!(results.len(), 2);
        assert_eq!(stats.timed_out, 1);
    }
}
