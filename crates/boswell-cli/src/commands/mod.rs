//! Command implementations.

pub mod assert;
pub mod connect;
pub mod forget;
pub mod learn;
pub mod profile;
pub mod query;
pub mod search;

pub use self::assert::execute_assert;
pub use self::connect::execute_connect;
pub use self::forget::execute_forget;
pub use self::learn::execute_learn;
pub use self::profile::execute_profile;
pub use self::query::execute_query;
pub use self::search::execute_search;
