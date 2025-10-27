use alloy::providers::Provider;
use alloy::rpc::types::Log;
use async_trait::async_trait;
use eyre::Result;

use crate::types::{PoolClass, PoolData, PoolId};

/// 池子加载器 Trait - 简化版本
/// 
/// 使用 async-trait 简化异步方法定义，去除 Pin<Box<dyn Future>>
#[async_trait]
pub trait PoolLoader: Send + Sync {
    /// 从日志中识别池子类型
    /// 
    /// 返回 Some((pool_id, pool_class)) 如果日志匹配该池子类型
    fn identify_pool_from_log(&self, log: &Log) -> Option<(PoolId, PoolClass)>;
    
    /// 从链上加载池子完整数据
    /// 
    /// 使用 async-trait，不需要手动 Pin<Box>
    async fn fetch_pool_data<P>(&self, provider: &P, pool_id: PoolId) -> Result<PoolData>
    where
        P: Provider + Clone + 'static;
}
