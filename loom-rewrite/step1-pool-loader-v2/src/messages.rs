use crate::state_update::BlockStateUpdate;
use crate::types::{PoolClass, PoolData, PoolId};

/// Actor 之间传递的消息类型
#[derive(Debug, Clone)]
pub enum Message {
    /// 请求加载池子
    /// Vec<(PoolId, PoolClass)> - 需要加载的池子列表
    FetchPools(Vec<(PoolId, PoolClass)>),
    
    /// 池子加载完成
    /// PoolData - 加载完成的池子数据
    PoolLoaded(PoolData),
    
    /// 区块状态更新
    /// BlockStateUpdate - 区块的状态变化
    BlockStateUpdate(BlockStateUpdate),
    
    /// 停止 Actor
    Stop,
}
