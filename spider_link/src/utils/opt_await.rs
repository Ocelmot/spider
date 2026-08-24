//! Helper implementation for optional futures that pend instead of return

use std::future::Future;

use futures::future::pending;


/// Implements helper functions for optional futures
pub trait OrPend<T>{
    /// polls Some(future), while pending() on None. This is the opposite of OptionFuture
    fn or_pend(self) -> impl Future<Output = T::Output> where T:Future;
}

impl<T> OrPend<T> for Option<T> {
    fn or_pend(self) -> impl Future<Output = T::Output> where T:Future {
        async move{
            match self {
                Some(f) => f.await,
                None => pending().await,
            }
        }
    }
}

