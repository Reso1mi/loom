use alloy::primitives::Address;
use eyre::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::actors::Actor;
use crate::market_state::MarketState;
use crate::messages::Message;
use crate::state_update::BlockStateUpdate;

/// 状态更新处理器 Actor
/// 
/// 职责:
/// 1. 接收区块状态更新消息
/// 2. 分析受影响的池子
/// 3. 触发套利搜索 (未来扩展)
/// 4. 记录统计信息
pub struct StateUpdateProcessor {
    /// 市场状态 (共享)
    market_state: Arc<RwLock<MarketState>>,
    /// 消息接收通道
    message_rx: broadcast::Receiver<Message>,
    /// 统计信息
    stats: Arc<RwLock<ProcessorStats>>,
}

#[derive(Debug, Default)]
struct ProcessorStats {
    /// 处理的区块数
    blocks_processed: u64,
    /// 受影响的池子总数
    total_affected_pools: usize,
    /// 最后处理的区块
    last_block: u64,
}

impl StateUpdateProcessor {
    pub fn new(
        market_state: Arc<RwLock<MarketState>>,
        message_rx: broadcast::Receiver<Message>,
    ) -> Self {
        Self {
            market_state,
            message_rx,
            stats: Arc::new(RwLock::new(ProcessorStats::default())),
        }
    }
    
    /// Worker 函数 - 处理状态更新
    async fn run_worker(mut self) -> Result<()> {
        info!("StateUpdateProcessor started");
        
        loop {
            match self.message_rx.recv().await {
                Ok(Message::BlockStateUpdate(block_state_update)) => {
                    if let Err(e) = self.process_state_update(block_state_update).await {
                        error!("Failed to process state update: {}", e);
                    }
                }
                Ok(Message::Stop) => {
                    info!("StateUpdateProcessor received stop signal");
                    break;
                }
                Ok(_) => {
                    // 忽略其他消息
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    error!("StateUpdateProcessor lagged, skipped {} messages", skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Message channel closed, stopping StateUpdateProcessor");
                    break;
                }
            }
        }
        
        // 打印最终统计
        let stats = self.stats.read().await;
        info!(
            "StateUpdateProcessor finished. Stats: blocks={}, affected_pools={}, last_block={}",
            stats.blocks_processed,
            stats.total_affected_pools,
            stats.last_block
        );
        
        Ok(())
    }
    
    /// 处理单个状态更新
    async fn process_state_update(&self, update: BlockStateUpdate) -> Result<()> {
        let block_number = update.block_number;
        
        debug!("Processing state update for block {}", block_number);
        
        // 1. 获取受影响的池子
        let market_state = self.market_state.read().await;
        let affected_pools = market_state.get_affected_pools_vec(&update.state_update);
        drop(market_state);
        
        if affected_pools.is_empty() {
            debug!("No affected pools in block {}", block_number);
            return Ok(());
        }
        
        info!(
            "Block {} affected {} pools: {:?}",
            block_number,
            affected_pools.len(),
            affected_pools
        );
        
        // 2. 更新统计信息
        let mut stats = self.stats.write().await;
        stats.blocks_processed += 1;
        stats.total_affected_pools += affected_pools.len();
        stats.last_block = block_number;
        drop(stats);
        
        // 3. 分析每个受影响的池子
        for pool_address in affected_pools {
            self.analyze_pool_changes(pool_address, &update).await?;
        }
        
        Ok(())
    }
    
    /// 分析单个池子的变化
    async fn analyze_pool_changes(
        &self,
        pool_address: Address,
        update: &BlockStateUpdate,
    ) -> Result<()> {
        let market_state = self.market_state.read().await;
        
        // 获取池子信息
        let pool = match market_state.get_pool(&pool_address) {
            Some(pool) => pool,
            None => {
                debug!("Pool {} not found in market state", pool_address);
                return Ok(());
            }
        };
        
        debug!(
            "Analyzing pool changes: address={:?}, class={:?}",
            pool_address,
            pool.pool_class()
        );
        
        // 获取该池子的 storage 变化
        for state_update in update.state_update.iter() {
            if let Some(account_state) = state_update.get(&pool_address) {
                debug!(
                    "Pool {} storage changes: {} slots",
                    pool_address,
                    account_state.storage.len()
                );
                
                // 这里可以进一步分析具体的 storage 变化
                // 例如: UniswapV2 的 reserves, UniswapV3 的 tick 等
                
                // TODO: 触发套利搜索
                // self.trigger_arbitrage_search(pool, account_state).await?;
            }
        }
        
        Ok(())
    }
    
    /// 获取统计信息
    pub async fn get_stats(&self) -> ProcessorStats {
        self.stats.read().await.clone()
    }
}

impl Actor for StateUpdateProcessor {
    fn start(self) -> Result<JoinHandle<Result<()>>> {
        Ok(tokio::spawn(async move {
            self.run_worker().await
        }))
    }
    
    fn name(&self) -> &'static str {
        "StateUpdateProcessor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_state::MarketState;
    use crate::state_update::StateUpdateVec;
    use alloy::primitives::B256;
    
    #[tokio::test]
    async fn test_processor_creation() {
        let market_state = Arc::new(RwLock::new(MarketState::new()));
        let (tx, rx) = broadcast::channel(10);
        
        let processor = StateUpdateProcessor::new(market_state, rx);
        assert_eq!(processor.name(), "StateUpdateProcessor");
    }
}
