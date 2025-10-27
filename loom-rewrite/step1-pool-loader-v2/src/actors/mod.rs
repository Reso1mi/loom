mod block_state_actor;
mod history_loader_actor;
mod pool_loader_actor;
mod state_update_processor;

pub use block_state_actor::BlockStateActor;
pub use history_loader_actor::HistoryPoolLoaderActor;
pub use pool_loader_actor::PoolLoaderActor;
pub use state_update_processor::StateUpdateProcessor;

use eyre::Result;
use tokio::task::JoinHandle;

/// Actor trait - 所有 Actor 必须实现
pub trait Actor {
    /// 启动 Actor,返回 JoinHandle
    fn start(self) -> Result<JoinHandle<Result<()>>>;
    
    /// Actor 名称
    fn name(&self) -> &'static str;
}
