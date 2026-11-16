//! Parallel when the `parallel` feature is on, sequential otherwise (WebAssembly plugins).

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// `(0..n).map(f)` collected, in parallel when available.
#[cfg(feature = "parallel")]
pub fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    (0..n).into_par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
pub fn map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    (0..n).map(f).collect()
}

/// `map` over `0..n` whose results are concatenated.
pub fn flat_map<T: Send>(n: usize, f: impl Fn(usize) -> Vec<T> + Sync + Send) -> Vec<T> {
    map(n, f).into_iter().flatten().collect()
}

/// How many threads to split work for.
pub fn threads() -> usize {
    #[cfg(feature = "parallel")]
    {
        rayon::current_num_threads()
    }
    #[cfg(not(feature = "parallel"))]
    {
        1
    }
}
