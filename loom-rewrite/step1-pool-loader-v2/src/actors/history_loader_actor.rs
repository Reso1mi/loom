use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use eyre::Result;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use super::Actor;
use crate::messages::Message;
use crate::pool_loaders::PoolLoaders;
use crate::types::LoaderConfig;

/// 历史池子加载 Actor - 简化版本
/// 
/// 功能：
/// 1. 从历史区块扫描日志
/// 2. 识别池子创建/交易事件
/// 3. 发送池子加载任务到消息通道
pub struct HistoryPoolLoaderActor<P> {
    /// RPC Provider
    provider: P,
    /// 池子加载器容器
    pool_loaders: Arc<PoolLoaders>,
    /// 配置
    config: LoaderConfig,
    /// 消息发送通道
    message_tx: broadcast::Sender<Message>,
}

impl<P> HistoryPoolLoaderActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    pub fn new(
        provider: P,
        pool_loaders: Arc<PoolLoaders>,
        config: LoaderConfig,
        message_tx: broadcast::Sender<Message>,
    ) -> Self {
        Self {
            provider,
            pool_loaders,
            config,
            message_tx,
        }
    }
    
    /// Worker 函数 - 实际执行扫描逻辑
    async fn run_worker(self) -> Result<()> {
        info!("HistoryPoolLoaderActor started");
        
        // 获取起始区块
        let mut current_block = if self.config.start_block == 0 {
            self.provider.get_block_number().await?
        } else {
            self.config.start_block
        };
        
        let block_batch_size = self.config.block_batch_size;
        
        // 循环扫描历史区块
        for batch_idx in 0..self.config.num_batches {
            if current_block < block_batch_size + 1 {
                info!("Reached genesis block, stopping");
                break;
            }
            
            current_block -= block_batch_size;
            
            debug!(
                "Scanning blocks {} to {} (batch {}/{})",
                current_block,
                current_block + block_batch_size - 1,
                batch_idx + 1,
                self.config.num_batches
            );
            
            // 创建日志过滤器
            let filter = Filter::new()
                .from_block(current_block)
                .to_block(current_block + block_batch_size - 1);
            
            // 获取日志
            match self.provider.get_logs(&filter).await {
                Ok(logs) => {
                    debug!("Got {} logs", logs.len());
                    
                    // 处理日志
                    if let Err(e) = self.process_logs(logs).await {
                        error!("Error processing logs: {}", e);
                    }
                }
                Err(e) => {
                    error!("Error fetching logs: {}", e);
                }
            }
        }
        
        info!("HistoryPoolLoaderActor finished");
        Ok(())
    }
    
    /// 处理日志，识别池子
    async fn process_logs(&self, logs: Vec<alloy::rpc::types::Log>) -> Result<()> {
        use std::collections::{HashMap, HashSet};
        
        let mut pools_to_fetch = Vec::new();
        let mut seen_addresses = HashSet::new();
        
        // 遍历日志，识别池子
        for log in logs {
            // 使用 PoolLoaders 识别池子类型
            if let Some((pool_id, pool_class)) = self.pool_loaders.identify_pool_from_log(&log) {
                // 去重：同一个地址只处理一次
                if seen_addresses.insert(pool_id.address()) {
                    pools_to_fetch.push((pool_id, pool_class));
                }
            }
        }
        
        if !pools_to_fetch.is_empty() {
            debug!("Found {} unique pools to fetch", pools_to_fetch.len());
            
            // 发送消息到 PoolLoaderActor
            self.message_tx.send(Message::FetchPools(pools_to_fetch))?;
        }
        
        Ok(())
    }
}

impl<P> Actor for HistoryPoolLoaderActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    fn start(self) -> Result<JoinHandle<Result<()>>> {
        Ok(tokio::spawn(self.run_worker()))
    }
    
    fn name(&self) -> &'static str {
        "HistoryPoolLoaderActor"
    }
}
