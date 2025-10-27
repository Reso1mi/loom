use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockId, BlockNumberOrTag};
use eyre::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::actors::Actor;
use crate::market_state::MarketState;
use crate::messages::Message;
use crate::state_update::{BlockStateUpdate, StateUpdate, StateUpdateVec};

/// 区块状态更新 Actor
/// 
/// 职责:
/// 1. 监听新区块
/// 2. 获取区块的状态更新 (通过 debug_traceBlock)
/// 3. 应用状态更新到 MarketState
/// 4. 识别受影响的池子
/// 5. 广播状态更新事件
pub struct BlockStateActor<P> {
    /// RPC Provider
    provider: P,
    /// 市场状态 (共享)
    market_state: Arc<RwLock<MarketState>>,
    /// 消息发送通道
    message_tx: broadcast::Sender<Message>,
    /// 轮询间隔 (秒)
    poll_interval_secs: u64,
}

impl<P> BlockStateActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    pub fn new(
        provider: P,
        market_state: Arc<RwLock<MarketState>>,
        message_tx: broadcast::Sender<Message>,
        poll_interval_secs: u64,
    ) -> Self {
        Self {
            provider,
            market_state,
            message_tx,
            poll_interval_secs,
        }
    }
    
    /// Worker 函数 - 监听新区块并处理状态更新
    async fn run_worker(self) -> Result<()> {
        info!("BlockStateActor started with poll_interval={}s", self.poll_interval_secs);
        
        let mut last_processed_block = 0u64;
        
        loop {
            // 获取最新区块号
            let latest_block = match self.provider.get_block_number().await {
                Ok(block) => block,
                Err(e) => {
                    error!("Failed to get latest block number: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(self.poll_interval_secs)).await;
                    continue;
                }
            };
            
            // 如果是第一次运行,从当前区块开始
            if last_processed_block == 0 {
                last_processed_block = latest_block;
                info!("Starting from block {}", latest_block);
            }
            
            // 处理所有新区块
            while last_processed_block < latest_block {
                let block_to_process = last_processed_block + 1;
                
                match self.process_block(block_to_process).await {
                    Ok(affected_pools) => {
                        if !affected_pools.is_empty() {
                            info!(
                                "Block {} processed, {} pools affected",
                                block_to_process,
                                affected_pools.len()
                            );
                        } else {
                            debug!("Block {} processed, no pools affected", block_to_process);
                        }
                        
                        last_processed_block = block_to_process;
                    }
                    Err(e) => {
                        error!("Failed to process block {}: {}", block_to_process, e);
                        // 继续处理下一个区块
                        last_processed_block = block_to_process;
                    }
                }
            }
            
            // 等待下一个轮询周期
            tokio::time::sleep(tokio::time::Duration::from_secs(self.poll_interval_secs)).await;
        }
    }
    
    /// 处理单个区块
    async fn process_block(&self, block_number: u64) -> Result<Vec<Address>> {
        // 1. 获取区块信息
        let block = self.provider
            .get_block_by_number(BlockNumberOrTag::Number(block_number), false)
            .await?
            .ok_or_else(|| eyre::eyre!("Block {} not found", block_number))?;
        
        let block_hash = block.header.hash;
        
        // 2. 获取状态更新
        // 注意: 这里需要调用 debug_traceBlock
        // 由于 alloy 可能没有直接支持,我们先用模拟数据
        // 实际项目中需要实现 debug_traceBlock 调用
        let state_update = self.fetch_state_update(block_number, block_hash).await?;
        
        if state_update.is_empty() {
            return Ok(Vec::new());
        }
        
        // 3. 应用状态更新到 MarketState
        let mut market_state = self.market_state.write().await;
        market_state.apply_state_update_vec(&state_update);
        market_state.update_block(block_number);
        
        // 4. 识别受影响的池子
        let affected_pools = market_state.get_affected_pools_vec(&state_update);
        
        drop(market_state);
        
        // 5. 广播状态更新事件
        if !affected_pools.is_empty() {
            let block_state_update = BlockStateUpdate::new(
                block_number,
                block_hash,
                state_update,
            );
            
            if let Err(e) = self.message_tx.send(Message::BlockStateUpdate(block_state_update)) {
                warn!("Failed to broadcast block state update: {}", e);
            }
        }
        
        Ok(affected_pools)
    }
    
    /// 获取区块的状态更新
    /// 
    /// 注意: 这是一个简化版本
    /// 实际项目中需要调用 debug_traceBlock RPC 方法
    async fn fetch_state_update(
        &self,
        _block_number: u64,
        _block_hash: B256,
    ) -> Result<StateUpdateVec> {
        // TODO: 实现 debug_traceBlock 调用
        // 
        // 实际调用示例:
        // let result = self.provider
        //     .raw_request::<_, Vec<TraceResult>>(
        //         "debug_traceBlock",
        //         (block_hash, json!({"tracer": "prestateTracer"}))
        //     )
        //     .await?;
        // 
        // 然后解析 result 转换为 StateUpdateVec
        
        // 暂时返回空的状态更新
        Ok(Vec::new())
    }
}

impl<P> Actor for BlockStateActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    fn start(self) -> Result<JoinHandle<Result<()>>> {
        Ok(tokio::spawn(async move {
            self.run_worker().await
        }))
    }
    
    fn name(&self) -> &'static str {
        "BlockStateActor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    // 测试需要实际的 Provider,这里省略
}
