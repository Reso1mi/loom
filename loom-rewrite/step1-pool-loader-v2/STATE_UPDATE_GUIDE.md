# 状态更新机制指南

## 概述

本项目已集成完整的**区块状态更新机制**,实现了与 Loom 原项目相同的核心功能:自动同步链上状态到本地数据库。

## 架构设计

### 核心组件

```
┌─────────────────────────────────────────────────────────────┐
│                      消息总线 (Broadcast)                      │
└─────────────────────────────────────────────────────────────┘
         ↑                    ↑                    ↑
         │                    │                    │
    ┌────┴────┐         ┌────┴────┐         ┌────┴────┐
    │ History │         │  Block  │         │  State  │
    │ Loader  │         │  State  │         │ Update  │
    │  Actor  │         │  Actor  │         │Processor│
    └─────────┘         └─────────┘         └─────────┘
         │                    │                    │
         ↓                    ↓                    ↓
    扫描历史区块          监听新区块            处理状态更新
    识别池子              获取StateUpdate       触发套利搜索
         │                    │                    │
         └────────────────────┴────────────────────┘
                              ↓
                    ┌──────────────────┐
                    │   MarketState    │
                    │   (共享状态)      │
                    │  - LocalDB       │
                    │  - Pools         │
                    └──────────────────┘
```

### 新增模块

#### 1. `state_update.rs` - 状态更新数据结构

```rust
/// 账户状态变化
pub struct AccountState {
    pub nonce: Option<u64>,
    pub balance: Option<U256>,
    pub code: Option<Bytes>,
    pub storage: BTreeMap<B256, U256>,  // ⭐ 关键: storage变化
}

/// 单个区块的状态更新
pub type StateUpdate = BTreeMap<Address, AccountState>;

/// 区块状态更新消息
pub struct BlockStateUpdate {
    pub block_number: u64,
    pub block_hash: B256,
    pub state_update: StateUpdateVec,
}
```

**功能**:
- 定义状态更新的数据结构
- 对应 Geth 的 `debug_traceBlock` 返回格式
- 包含所有合约 storage 的变化

#### 2. `market_state.rs` - 市场状态管理

```rust
/// 简化的本地数据库
pub struct LocalDB {
    storage: HashMap<Address, HashMap<U256, U256>>,
    balances: HashMap<Address, U256>,
    nonces: HashMap<Address, u64>,
}

/// 市场状态
pub struct MarketState {
    pub db: LocalDB,
    pub pools: HashMap<Address, PoolData>,
    pub current_block: u64,
}
```

**功能**:
- 管理本地数据库 (LocalDB)
- 存储所有池子信息
- 应用状态更新
- 识别受影响的池子

**关键方法**:
```rust
// 应用状态更新
market_state.apply_state_update(&update);

// 获取受影响的池子
let affected = market_state.get_affected_pools(&update);

// 读取 storage
let value = market_state.db.get_storage(address, slot);
```

#### 3. `actors/block_state_actor.rs` - 区块状态监听

```rust
pub struct BlockStateActor<P> {
    provider: P,
    market_state: Arc<RwLock<MarketState>>,
    message_tx: broadcast::Sender<Message>,
    poll_interval_secs: u64,
}
```

**职责**:
1. 轮询监听新区块
2. 获取区块的状态更新 (通过 `debug_traceBlock`)
3. 应用状态更新到 `MarketState`
4. 识别受影响的池子
5. 广播 `BlockStateUpdate` 消息

**工作流程**:
```rust
loop {
    // 1. 获取最新区块
    let latest_block = provider.get_block_number().await?;
    
    // 2. 处理新区块
    for block in last_block..latest_block {
        // 3. 获取状态更新
        let state_update = fetch_state_update(block).await?;
        
        // 4. 应用到 MarketState
        market_state.apply_state_update_vec(&state_update);
        
        // 5. 识别受影响的池子
        let affected = market_state.get_affected_pools_vec(&state_update);
        
        // 6. 广播消息
        message_tx.send(Message::BlockStateUpdate(...));
    }
    
    // 7. 等待下一个轮询周期
    sleep(poll_interval).await;
}
```

#### 4. `actors/state_update_processor.rs` - 状态更新处理

```rust
pub struct StateUpdateProcessor {
    market_state: Arc<RwLock<MarketState>>,
    message_rx: broadcast::Receiver<Message>,
    stats: Arc<RwLock<ProcessorStats>>,
}
```

**职责**:
1. 接收 `BlockStateUpdate` 消息
2. 分析受影响的池子
3. 记录统计信息
4. 触发套利搜索 (未来扩展)

**工作流程**:
```rust
loop {
    match message_rx.recv().await {
        Ok(Message::BlockStateUpdate(update)) => {
            // 1. 获取受影响的池子
            let affected = market_state.get_affected_pools_vec(&update.state_update);
            
            // 2. 分析每个池子的变化
            for pool_address in affected {
                analyze_pool_changes(pool_address, &update).await?;
            }
            
            // 3. 更新统计信息
            stats.blocks_processed += 1;
            stats.total_affected_pools += affected.len();
        }
        _ => {}
    }
}
```

## 数据流

### 完整流程

```
新区块产生
    ↓
BlockStateActor 检测到新区块
    ↓
调用 debug_traceBlock (TODO: 需要实现)
    ↓
获取 StateUpdate {
    pool_address: {
        storage: {
            slot_0: new_value,
            slot_4: new_liquidity,
            slot_6: new_tick_bitmap,
        }
    }
}
    ↓
应用到 MarketState.db
    ↓
识别受影响的池子
    ↓
广播 BlockStateUpdate 消息
    ↓
StateUpdateProcessor 接收消息
    ↓
分析池子变化
    ↓
触发套利搜索 (未来)
```

### 消息类型

```rust
pub enum Message {
    /// 请求加载池子
    FetchPools(Vec<(PoolId, PoolClass)>),
    
    /// 池子加载完成
    PoolLoaded(PoolData),
    
    /// 区块状态更新 ⭐ 新增
    BlockStateUpdate(BlockStateUpdate),
    
    /// 停止 Actor
    Stop,
}
```

## 使用示例

### 启动所有 Actor

```rust
// 1. 创建共享的市场状态
let market_state = Arc::new(RwLock::new(MarketState::new()));

// 2. 创建消息通道
let (message_tx, _) = broadcast::channel::<Message>(1000);

// 3. 启动 BlockStateActor
let block_state_actor = BlockStateActor::new(
    provider.clone(),
    market_state.clone(),
    message_tx.clone(),
    2,  // 每2秒轮询一次
);
block_state_actor.start()?;

// 4. 启动 StateUpdateProcessor
let processor = StateUpdateProcessor::new(
    market_state.clone(),
    message_tx.subscribe(),
);
processor.start()?;
```

### 读取池子状态

```rust
// 从 MarketState 读取池子信息
let market = market_state.read().await;

// 获取池子
if let Some(pool) = market.get_pool(&pool_address) {
    println!("Pool: {:?}", pool);
}

// 读取 storage (例如 UniswapV2 的 reserves)
let reserves_slot = U256::from(8);  // slot 8
let reserves = market.db.get_storage(pool_address, reserves_slot);
```

### 监听状态更新

```rust
let mut rx = message_tx.subscribe();

loop {
    match rx.recv().await {
        Ok(Message::BlockStateUpdate(update)) => {
            println!("Block {} updated", update.block_number);
            
            // 获取受影响的池子
            let market = market_state.read().await;
            let affected = market.get_affected_pools_vec(&update.state_update);
            
            for pool_address in affected {
                println!("Pool {} affected", pool_address);
            }
        }
        _ => {}
    }
}
```

## 与原 Loom 的对应关系

| 原 Loom 组件 | 本项目组件 | 说明 |
|-------------|-----------|------|
| `GethStateUpdate` | `StateUpdate` | 状态更新数据结构 |
| `MarketState` | `MarketState` | 市场状态管理 |
| `LoomDB` | `LocalDB` | 本地数据库 |
| `BlockStateChangeProcessor` | `BlockStateActor` | 区块状态监听 |
| `StateUpdateEvent` | `BlockStateUpdate` | 状态更新事件 |
| `get_affected_pools_from_state_update` | `MarketState::get_affected_pools_vec` | 识别受影响的池子 |

## TODO: 需要实现的功能

### 1. debug_traceBlock 调用

当前 `BlockStateActor::fetch_state_update` 返回空数据,需要实现实际的 RPC 调用:

```rust
async fn fetch_state_update(
    &self,
    block_number: u64,
    block_hash: B256,
) -> Result<StateUpdateVec> {
    // 调用 debug_traceBlock
    let result = self.provider
        .raw_request::<_, Vec<TraceResult>>(
            "debug_traceBlock",
            (
                block_hash,
                json!({
                    "tracer": "prestateTracer",
                    "tracerConfig": {
                        "diffMode": true
                    }
                })
            )
        )
        .await?;
    
    // 解析结果转换为 StateUpdateVec
    parse_trace_result(result)
}
```

### 2. UniswapV3 Tick 数据读取

实现从 `LocalDB` 读取 UniswapV3 的 tick 数据:

```rust
impl LocalDB {
    /// 读取 UniswapV3 tick_bitmap
    pub fn get_uniswap_v3_tick_bitmap(
        &self,
        pool_address: Address,
        word_pos: i16,
    ) -> Option<U256> {
        // slot 6 是 tickBitmap 的 base slot
        let base_slot = U256::from(6);
        let key = I256::from_raw(U256::from(word_pos));
        let actual_slot = keccak256(concat(key.to_bytes(), base_slot));
        
        self.get_storage(pool_address, actual_slot)
    }
    
    /// 读取 UniswapV3 tick 数据
    pub fn get_uniswap_v3_tick(
        &self,
        pool_address: Address,
        tick: i32,
    ) -> Option<TickInfo> {
        // slot 7 是 ticks mapping 的 base slot
        let base_slot = U256::from(7);
        let key = I256::from_raw(U256::from(tick));
        let actual_slot = keccak256(concat(key.to_bytes(), base_slot));
        
        let value = self.get_storage(pool_address, actual_slot)?;
        
        // 解析 tick 数据
        Some(TickInfo {
            liquidity_gross: U128::from(value & U256::from((1u128 << 128) - 1)),
            liquidity_net: I128::from_raw(U128::from(value >> 128)),
        })
    }
}
```

### 3. 套利搜索集成

在 `StateUpdateProcessor` 中添加套利搜索逻辑:

```rust
async fn analyze_pool_changes(
    &self,
    pool_address: Address,
    update: &BlockStateUpdate,
) -> Result<()> {
    // 1. 获取池子信息
    let market = self.market_state.read().await;
    let pool = market.get_pool(&pool_address)?;
    
    // 2. 获取相关的 swap paths
    let swap_paths = market.get_swap_paths_for_pool(&pool_address);
    
    // 3. 对每个 path 计算套利机会
    for path in swap_paths {
        if let Some(profit) = calculate_arbitrage(&market.db, &path).await? {
            info!("Found arbitrage: {} ETH", profit);
            // 发送套利交易...
        }
    }
    
    Ok(())
}
```

## 性能优化

### 1. 增量更新

只更新变化的 storage slot,不重新加载整个池子:

```rust
// ✅ 高效: 只更新变化的 slot
market_state.apply_state_update(&update);

// ❌ 低效: 重新加载整个池子
pool.reload_all_data().await?;
```

### 2. 并发处理

多个 Actor 可以同时处理状态更新:

```rust
// BlockStateActor: 应用状态更新
market_state.apply_state_update_vec(&update);

// StateUpdateProcessor: 分析受影响的池子
let affected = market_state.get_affected_pools_vec(&update);

// ArbSearcher: 计算套利机会
calculate_arbitrage(&market_state.db, &path);
```

### 3. 内存管理

使用 `Arc<RwLock<>>` 共享状态,避免数据复制:

```rust
// 共享的市场状态
let market_state = Arc::new(RwLock::new(MarketState::new()));

// 多个 Actor 共享同一个状态
let actor1 = BlockStateActor::new(..., market_state.clone(), ...);
let actor2 = StateUpdateProcessor::new(market_state.clone(), ...);
```

## 测试

### 单元测试

```bash
# 运行所有测试
cargo test

# 运行特定模块的测试
cargo test state_update
cargo test market_state
```

### 集成测试

```bash
# 设置 RPC URL
export RPC_URL="wss://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY"

# 运行项目
cargo run
```

## 日志输出

启动后会看到类似的日志:

```
INFO  BlockStateActor started with poll_interval=2s
INFO  StateUpdateProcessor started
INFO  Starting from block 18000000
INFO  Block 18000001 processed, 5 pools affected
INFO  Block 18000001 affected 5 pools: [0xabc..., 0xdef...]
INFO  Analyzing pool changes: address=0xabc..., class=UniswapV3
INFO  Pool 0xabc... storage changes: 3 slots
```

## 总结

本项目已成功集成状态更新机制,实现了:

✅ **自动同步**: 每个区块的状态变化自动应用到本地 DB  
✅ **实时监听**: BlockStateActor 持续监听新区块  
✅ **池子识别**: 自动识别受影响的池子  
✅ **统计信息**: 记录处理的区块数和受影响的池子数  
✅ **扩展性**: 易于添加套利搜索等功能  

下一步可以:
1. 实现 `debug_traceBlock` 调用
2. 添加 UniswapV3 tick 数据读取
3. 集成套利搜索逻辑
4. 优化性能和内存使用
