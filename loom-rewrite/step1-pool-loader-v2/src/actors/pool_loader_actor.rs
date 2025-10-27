use alloy::providers::Provider;
use eyre::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Semaphore};
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use super::Actor;
use crate::messages::Message;
use crate::pool_loaders::PoolLoaders;
use crate::types::{PoolClass, PoolData, PoolId};

/// 池子加载 Actor - 简化版本
/// 
/// 功能：
/// 1. 接收池子加载任务
/// 2. 并发加载池子数据
/// 3. 发送加载完成的消息
pub struct PoolLoaderActor<P> {
    /// RPC Provider
    provider: P,
    /// 池子加载器容器
    pool_loaders: Arc<PoolLoaders>,
    /// 最大并发数
    max_concurrent: usize,
    /// 消息接收通道
    message_rx: broadcast::Receiver<Message>,
    /// 消息发送通道（用于发送加载完成的消息）
    message_tx: broadcast::Sender<Message>,
}

impl<P> PoolLoaderActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    pub fn new(
        provider: P,
        pool_loaders: Arc<PoolLoaders>,
        max_concurrent: usize,
        message_rx: broadcast::Receiver<Message>,
        message_tx: broadcast::Sender<Message>,
    ) -> Self {
        Self {
            provider,
            pool_loaders,
            max_concurrent,
            message_rx,
            message_tx,
        }
    }
    
    /// Worker 函数 - 实际执行加载逻辑
    async fn run_worker(mut self) -> Result<()> {
        info!("PoolLoaderActor started with max_concurrent={}", self.max_concurrent);
        
        // 用于去重的 HashMap
        let mut processed_pools: HashMap<PoolId, bool> = HashMap::new();
        
        // 并发控制
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent));
        
        // 接收消息循环
        loop {
            match self.message_rx.recv().await {
                Ok(Message::FetchPools(pools)) => {
                    debug!("Received request to fetch {} pools", pools.len());
                    
                    for (pool_id, pool_class) in pools {
                        // 去重检查
                        if processed_pools.insert(pool_id, true).is_some() {
                            debug!("Pool {} already processed, skipping", pool_id.address());
                            continue;
                        }
                        
                        // 克隆需要的变量
                        let provider = self.provider.clone();
                        let pool_loaders = self.pool_loaders.clone();
                        let message_tx = self.message_tx.clone();
                        let semaphore = semaphore.clone();
                        
                        // 生成异步任务
                        tokio::spawn(async move {
                            // 获取信号量许可
                            let _permit = semaphore.acquire().await.unwrap();
                            
                            // 加载池子数据
                            match Self::fetch_pool(
                                &provider,
                                &pool_loaders,
                                pool_id,
                                pool_class,
                            ).await {
                                Ok(pool_data) => {
                                    info!(
                                        "Successfully loaded pool {} ({:?})",
                                        pool_data.address(),
                                        pool_class
                                    );
                                    
                                    // 发送加载完成消息
                                    let _ = message_tx.send(Message::PoolLoaded(pool_data));
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to load pool {} ({:?}): {}",
                                        pool_id.address(),
                                        pool_class,
                                        e
                                    );
                                }
                            }
                            
                            // _permit 自动释放
                        });
                    }
                }
                Ok(Message::Stop) => {
                    info!("Received stop signal");
                    break;
                }
                Ok(_) => {
                    // 忽略其他消息
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    error!("Message channel lagged by {} messages", n);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("Message channel closed");
                    break;
                }
            }
        }
        
        info!("PoolLoaderActor finished");
        Ok(())
    }
    
    /// 加载单个池子的数据
    async fn fetch_pool(
        provider: &P,
        pool_loaders: &PoolLoaders,
        pool_id: PoolId,
        pool_class: PoolClass,
    ) -> Result<PoolData> {
        // 获取对应的加载器
        let loader = pool_loaders
            .get_loader(&pool_class)
            .ok_or_else(|| eyre::eyre!("No loader found for pool class {:?}", pool_class))?;
        
        // 调用加载器获取数据
        loader.fetch_pool_data(provider, pool_id).await
    }
}

impl<P> Actor for PoolLoaderActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    fn start(self) -> Result<JoinHandle<Result<()>>> {
        Ok(tokio::spawn(self.run_worker()))
    }
    
    fn name(&self) -> &'static str {
        "PoolLoaderActor"
    }
}
