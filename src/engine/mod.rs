pub mod blocklist_manager;
pub mod domain_matcher;
pub mod state;
mod traits;

pub use blocklist_manager::StandardManager;
pub use domain_matcher::HashedMatcher;
pub use state::AppState;
pub use traits::{BlocklistManager, BlocklistMatcher};
