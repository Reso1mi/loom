mod actors;
mod market_state;
mod messages;
mod pool_loader_trait;
mod pool_loaders;
mod state_update;
mod types;
mod uniswap_v2_loader;
mod uniswap_v3_loader;

use alloy::providers::{ProviderBuilder, WsConnect};
use eyre::Result;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use actors::{Actor, BlockStateActor, HistoryPoolLoaderActor, PoolLoaderActor, StateUpdateProcessor};
use market_state::MarketState;
use messages::Message;
use pool_loaders::PoolLoaders;
use types::{LoaderConfig, PoolClass, PoolData};
use uniswap_v2_loader::UniswapV2Loader;
use uniswap_v3_loader::UniswapV3Loader;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    info!("🚀 Loom Pool Loader (Simplified Architecture)");
    
    // 1. 获取 RPC URL
    let rpc_url = env::var("RPC_URL")
        .unwrap_or_else(|_| "wss://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY".to_string());
    
    info!("Connecting to RPC: {}", rpc_url);
    
    // 2. 创建 Provider
    let ws = WsConnect::new(rpc_url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;
    
    info!("✅ Connected to Ethereum node");
    
    // 3. 构建 PoolLoaders
    let pool_loaders = Arc::new(
        PoolLoaders::new()
            .add_loader(PoolClass::UniswapV2, Arc::new(UniswapV2Loader::new()))
            .add_loader(PoolClass::UniswapV3, Arc::new(UniswapV3Loader::new()))
    );
    
    info!("✅ Initialized pool loaders: {:?}", pool_loaders.supported_pool_classes());
    
    // 4. 创建共享的市场状态
    let market_state = Arc::new(RwLock::new(MarketState::new()));
    info!("✅ Initialized market state");
    
    // 5. 创建消息通道
    let (message_tx, _) = broadcast::channel::<Message>(1000);
    
    // 6. 配置
    let config = LoaderConfig {
        max_concurrent_tasks: 20,
        start_block: 0,  // 从最新区块开始
        block_batch_size: 5,
        num_batches: 100,  // 扫描 100 批次（500 个区块）
    };
    
    info!("📋 Config: {:?}", config);
    
    // 6. 创建并启动 PoolLoaderActor
    let pool_loader_actor = PoolLoaderActor::new(
        provider.clone(),
        pool_loaders.clone(),
        config.max_concurrent_tasks,
        message_tx.subscribe(),
        message_tx.clone(),
    );
    
    info!("Starting PoolLoaderActor...");
    let pool_loader_handle = pool_loader_actor.start()?;
    
    // 8. 创建并启动 HistoryPoolLoaderActor
    let history_loader_actor = HistoryPoolLoaderActor::new(
        provider.clone(),
        pool_loaders.clone(),
        config,
        message_tx.clone(),
    );
    
    info!("Starting HistoryPoolLoaderActor...");
    let history_loader_handle = history_loader_actor.start()?;
    
    // 9. 创建并启动 BlockStateActor (监听新区块并更新状态)
    let block_state_actor = BlockStateActor::new(
        provider.clone(),
        market_state.clone(),
        message_tx.clone(),
        2,  // 每2秒轮询一次新区块
    );
    
    info!("Starting BlockStateActor...");
    let block_state_handle = block_state_actor.start()?;
    
    // 10. 创建并启动 StateUpdateProcessor (处理状态更新)
    let state_update_processor = StateUpdateProcessor::new(
        market_state.clone(),
        message_tx.subscribe(),
    );
    
    info!("Starting StateUpdateProcessor...");
    let state_processor_handle = state_update_processor.start()?;
    
    // 11. 启动结果收集器
    let mut result_rx = message_tx.subscribe();
    let market_state_clone = market_state.clone();
    let collector_handle = tokio::spawn(async move {
        let mut pools: HashMap<alloy::primitives::Address, PoolData> = HashMap::new();
        
        loop {
            match result_rx.recv().await {
                Ok(Message::PoolLoaded(pool_data)) => {
                    let address = pool_data.address();
                    pools.insert(address, pool_data.clone());
                    
                    // 同时添加到 MarketState
                    market_state_clone.write().await.add_pool(pool_data);
                    
                    if pools.len() % 10 == 0 {
                        info!("📊 Loaded {} pools so far", pools.len());
                    }
                }
                Ok(Message::Stop) => {
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        
        info!("✅ Total pools loaded: {}", pools.len());
        
        // 打印市场状态统计
        let market_stats = market_state_clone.read().await.stats();
        info!("📊 Market State Stats: {:?}", market_stats);
        
        // 保存到文件
        if !pools.is_empty() {
            let pools_vec: Vec<_> = pools.into_values().collect();
            let json = serde_json::to_string_pretty(&pools_vec).unwrap();
            std::fs::write("pools.json", json).unwrap();
            info!("💾 Saved pools to pools.json");
        }
    });
    
    // 12. 等待 HistoryPoolLoaderActor 完成
    info!("⏳ Waiting for history loader to finish...");
    history_loader_handle.await??;
    
    // 13. 等待一段时间让 PoolLoaderActor 处理完所有任务
    info!("⏳ Waiting for pool loader to finish processing...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    
    // 14. 发送停止信号
    message_tx.send(Message::Stop)?;
    
    // 15. 等待所有 Actor 完成
    pool_loader_handle.await??;
    block_state_handle.await??;
    state_processor_handle.await??;
    collector_handle.await?;
    
    info!("🎉 All done!");
    
    Ok(())
}
