# 池子状态更新代码示例

## 完整代码流程示例

### 1. 节点层 - 获取StateUpdate

```rust
// crates/node/json-rpc/src/node_block_state_update_worker.rs
use loom_node_debug_provider::DebugProviderExt;

pub async fn new_node_block_state_update_worker<N, P>(
    client: P,
    block_header_receiver: Broadcaster<Header>,
    sender: Broadcaster<MessageBlockStateUpdate>,
) -> WorkerResult 
where
    P: Provider<N> + DebugProviderExt<N> + Send + Sync + 'static,
{
    subscribe!(block_header_receiver);
    
    loop {
        if let Ok(block_header) = block_header_receiver.recv().await {
            let block_hash = block_header.hash;
            
            // 🔑 关键调用: debug_traceBlock 获取完整状态变化
            match debug_trace_block(
                client.clone(), 
                BlockId::Hash(block_hash.into()), 
                true  // prestate_tracer
            ).await {
                Ok((_, state_update)) => {
                    // state_update 包含所有storage变化
                    let msg = BlockStateUpdate {
                        block_header,
                        state_update,  // Vec<BTreeMap<Address, AccountState>>
                    };
                    
                    sender.send(Message::new_with_time(msg));
                }
                Err(e) => {
                    error!("Failed to get state update: {}", e);
                }
            }
        }
    }
}
```

### 2. 数据结构 - StateUpdate详解

```rust
// crates/types/blockchain/src/state_update.rs

/// 单个区块的状态更新
pub type GethStateUpdate = BTreeMap<Address, AccountState>;

/// 多个交易的状态更新向量
pub type GethStateUpdateVec = Vec<BTreeMap<Address, AccountState>>;

/// 账户状态变化
#[derive(Clone, Debug, Default)]
pub struct AccountState {
    pub nonce: Option<u64>,
    pub balance: Option<U256>,
    pub code: Option<Bytes>,
    pub storage: BTreeMap<B256, U256>,  // ⭐ slot -> value
}

/// 示例: UniswapV3池子的StateUpdate
/// 
/// 假设池子地址: 0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640 (USDC/WETH)
/// 用户添加流动性后的state_update:
/// 
/// GethStateUpdate {
///     0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640: AccountState {
///         nonce: None,
///         balance: None,
///         code: None,
///         storage: {
///             // slot 0: sqrtPriceX96, tick, observationIndex, etc.
///             0x0000000000000000000000000000000000000000000000000000000000000000: 
///                 U256::from_str("0x...new_slot0_value"),
///             
///             // slot 4: liquidity
///             0x0000000000000000000000000000000000000000000000000000000000000004:
///                 U256::from_str("0x...new_liquidity"),
///             
///             // slot 6 + keccak256(wordPos): tickBitmap
///             0xabcd1234...: U256::from_str("0x...new_bitmap"),
///             
///             // slot 7 + keccak256(tick): tick.liquidityGross, liquidityNet
///             0xef567890...: U256::from_str("0x...new_tick_data"),
///         }
///     }
/// }
```

### 3. MarketState层 - 应用StateUpdate

```rust
// crates/types/entities/src/market_state.rs

impl MarketState<DB> 
where 
    DB: Database + DatabaseRef + DatabaseCommit 
{
    /// 应用单个StateUpdate
    pub fn apply_geth_update(&mut self, update: GethStateUpdate) {
        DatabaseHelpers::apply_geth_state_update(&mut self.state_db, update)
    }
    
    /// 应用多个StateUpdate (一个区块内的多个交易)
    pub fn apply_geth_update_vec(&mut self, update: GethStateUpdateVec) {
        for entry in update {
            self.apply_geth_update(entry)
        }
    }
}

// crates/evm/db/src/loom_db_helper.rs
impl DatabaseHelpers {
    pub fn apply_geth_state_update<DB>(
        db: &mut DB, 
        update: GethStateUpdate
    ) 
    where 
        DB: Database + DatabaseCommit 
    {
        for (address, account_state) in update.iter() {
            // 更新nonce
            if let Some(nonce) = account_state.nonce {
                let mut account_info = db.basic(*address)
                    .unwrap()
                    .unwrap_or_default();
                account_info.nonce = nonce;
                db.insert_account_info(*address, account_info);
            }
            
            // 更新balance
            if let Some(balance) = account_state.balance {
                let mut account_info = db.basic(*address)
                    .unwrap()
                    .unwrap_or_default();
                account_info.balance = balance;
                db.insert_account_info(*address, account_info);
            }
            
            // 更新code
            if let Some(code) = &account_state.code {
                let bytecode = Bytecode::new_raw(code.clone());
                db.insert_account_info(*address, AccountInfo {
                    code_hash: bytecode.hash_slow(),
                    code: Some(bytecode),
                    ..Default::default()
                });
            }
            
            // 🔑 关键: 更新storage
            for (slot, value) in account_state.storage.iter() {
                let slot_u256 = U256::from_be_bytes(slot.0);
                db.insert_account_storage(*address, slot_u256, *value)
                    .unwrap();
            }
        }
    }
}
```

### 4. 策略层 - 识别受影响的池子

```rust
// crates/strategy/backrun/src/affected_pools_state.rs

/// 从StateUpdate中识别受影响的池子
pub async fn get_affected_pools_from_state_update(
    market: SharedState<Market>,
    state_update: &GethStateUpdateVec,
) -> BTreeMap<PoolWrapper, Vec<SwapDirection>> {
    let market_guard = market.read().await;
    let mut affected_pools = BTreeMap::new();
    
    // 遍历所有状态变化
    for state_update_record in state_update.iter() {
        for (address, state_update_entry) in state_update_record.iter() {
            
            // 情况1: 检查是否是pool manager (如UniswapV4)
            if market_guard.is_pool_manager(address) {
                // 遍历所有变化的storage slot
                for cell in state_update_entry.storage.keys() {
                    let cell_u = U256::from_be_slice(cell.as_slice());
                    
                    // 通过cell找到对应的pool_id
                    if let Some(pool_id) = market_guard.get_pool_id_for_cell(address, &cell_u) {
                        if let Some(pool) = market_guard.get_pool(&pool_id) {
                            if !affected_pools.contains_key(pool) {
                                debug!("Affected pool_manager {} pool {}", address, pool_id);
                                affected_pools.insert(
                                    pool.clone(), 
                                    pool.get_swap_directions()
                                );
                            }
                        }
                    }
                }
            } 
            // 情况2: 直接检查是否是已知池子地址
            else if let Some(pool) = market_guard.get_pool(&PoolId::Address(*address)) {
                if !affected_pools.contains_key(pool) {
                    debug!("Affected pool {}", address);
                    affected_pools.insert(
                        pool.clone(), 
                        pool.get_swap_directions()
                    );
                }
            }
        }
    }
    
    affected_pools
}
```

### 5. 策略层 - 处理状态更新

```rust
// crates/strategy/backrun/src/block_state_change_processor.rs

pub async fn block_state_change_processor<DB>(
    market: SharedState<Market>,
    block_history: SharedState<BlockHistory<DB>>,
    market_events_rx: Broadcaster<MarketEvents>,
    state_updates_broadcaster: Broadcaster<StateUpdateEvent<DB>>,
) -> WorkerResult 
where
    DB: Database + DatabaseRef + DatabaseCommit + Clone + Send + Sync + 'static,
{
    subscribe!(market_events_rx);
    
    loop {
        if let Ok(market_event) = market_events_rx.recv().await {
            match market_event {
                MarketEvents::BlockStateUpdate { block_hash } => {
                    // 1. 从block_history获取state_update
                    let block_history_guard = block_history.read().await;
                    let Some(block_history_entry) = block_history_guard.get_entry(&block_hash) 
                        else {
                            error!("Block {:?} not found in history", block_hash);
                            continue;
                        };
                    
                    let Some(state_update) = block_history_entry.state_update.clone() 
                        else {
                            error!("Block {:?} has no state update", block_hash);
                            continue;
                        };
                    
                    drop(block_history_guard);
                    
                    // 2. 识别受影响的池子
                    let affected_pools = get_affected_pools_from_state_update(
                        market.clone(), 
                        &state_update
                    ).await;
                    
                    if affected_pools.is_empty() {
                        debug!("No affected pools in block {:?}", block_hash);
                        continue;
                    }
                    
                    info!(
                        "Block {:?} affected {} pools", 
                        block_hash, 
                        affected_pools.len()
                    );
                    
                    // 3. 获取当前市场状态
                    let market_state = block_history.read().await
                        .get_market_state_by_hash(&block_hash)
                        .await?;
                    
                    // 4. 创建状态更新事件
                    let state_update_event = StateUpdateEvent {
                        origin: "block".to_string(),
                        market_state,
                        state_update,
                        directions: affected_pools,
                        stuffing_tx_hash: None,
                        tips_pct: None,
                    };
                    
                    // 5. 广播给套利搜索器
                    if let Err(e) = state_updates_broadcaster.send(state_update_event).await {
                        error!("Failed to broadcast state update: {}", e);
                    }
                }
                _ => {}
            }
        }
    }
}
```

### 6. 套利搜索层 - 使用最新状态

```rust
// crates/strategy/backrun/src/state_change_arb_searcher.rs

async fn state_change_arb_searcher_task<DB>(
    thread_pool: Arc<ThreadPool>,
    backrun_config: BackrunConfig,
    state_update_event: StateUpdateEvent<DB>,
    market: SharedState<Market>,
    swap_request_tx: Broadcaster<MessageSwapCompose<DB>>,
) -> Result<()> 
where
    DB: Database + DatabaseRef + DatabaseCommit + Clone + Send + Sync + 'static,
{
    debug!("Processing state update from {}", state_update_event.origin);
    
    // 1. 获取更新后的DB状态
    let mut db = state_update_event.market_state().clone();
    DatabaseHelpers::apply_geth_state_update_vec(
        &mut db, 
        state_update_event.state_update().clone()
    );
    
    // 2. 读取市场信息
    let market_guard = market.read().await;
    
    // 3. 遍历所有受影响的池子
    for (pool, directions) in state_update_event.directions().iter() {
        // 获取该池子相关的所有swap paths
        let pool_paths = match market_guard.get_pool_paths(&pool.get_pool_id()) {
            Some(paths) => paths,
            None => continue,
        };
        
        // 4. 对每个path计算套利机会
        for swap_path in pool_paths {
            // 🔑 关键: 使用更新后的db进行计算
            // 此时读取的tick数据已经是最新的!
            let result = calculate_arbitrage(
                &db,           // ← 包含最新tick数据的DB
                &swap_path,
                &backrun_config,
            );
            
            if let Some(profit) = result {
                info!("Found arbitrage opportunity: {} ETH", profit);
                
                // 发送套利请求
                swap_request_tx.send(MessageSwapCompose {
                    swap_path,
                    profit,
                    poststate: Some(db.clone()),
                    poststate_update: Some(state_update_event.state_update().clone()),
                    ...
                }).await?;
            }
        }
    }
    
    Ok(())
}

/// 套利计算函数
fn calculate_arbitrage<DB>(
    db: &DB,
    swap_path: &SwapPath,
    config: &BackrunConfig,
) -> Option<U256> 
where
    DB: DatabaseRef,
{
    // 创建EVM环境
    let mut evm = Evm::builder()
        .with_db(db)  // ← 使用最新状态的DB
        .build();
    
    // 模拟交换
    for pool in swap_path.pools.iter() {
        match pool {
            PoolWrapper::UniswapV3(pool) => {
                // 🔑 这里读取的tick数据是最新的!
                // 因为db已经通过apply_geth_update更新了
                let swap_result = pool.calculate_swap_return(
                    db,           // ← 从这里读取tick_bitmap和tick数据
                    amount_in,
                    token_in,
                    token_out,
                )?;
                
                amount_in = swap_result.amount_out;
            }
            _ => {}
        }
    }
    
    // 计算利润
    if amount_out > initial_amount {
        Some(amount_out - initial_amount)
    } else {
        None
    }
}
```

### 7. UniswapV3特定 - Tick数据读取

```rust
// crates/defi/pools/src/protocols/uniswap_v3/db_reader.rs

impl UniswapV3DBReader {
    /// 从DB读取tick_bitmap
    pub fn tick_bitmap<DB: DatabaseRef>(
        db: &DB,
        address: Address,
        word_pos: i16,
    ) -> Result<U256> {
        // slot 6 是 tickBitmap 的base slot
        let base_slot = U256::from(6);
        
        // 计算实际slot: keccak256(word_pos . base_slot)
        let key = I256::from_raw(U256::from(word_pos));
        let actual_slot = get_mapping_slot(key, base_slot);
        
        // 🔑 从DB读取 - 这里的值已经通过state_update更新了!
        let value = db.storage(address, actual_slot)?;
        
        Ok(value)
    }
    
    /// 从DB读取tick数据
    pub fn ticks<DB: DatabaseRef>(
        db: &DB,
        address: Address,
        tick: i32,
    ) -> Result<TickInfo> {
        // slot 7 是 ticks mapping的base slot
        let base_slot = U256::from(7);
        
        // 计算实际slot
        let key = I256::from_raw(U256::from(tick));
        let actual_slot = get_mapping_slot(key, base_slot);
        
        // 🔑 从DB读取tick信息
        let value = db.storage(address, actual_slot)?;
        
        // 解析tick数据
        let liquidity_gross = U128::from(value & U256::from((1u128 << 128) - 1));
        let liquidity_net = I128::from_raw(U128::from(value >> 128));
        
        Ok(TickInfo {
            liquidity_gross,
            liquidity_net,
            ...
        })
    }
}
```

---

## 完整时间线示例

```
T0: 用户在UniswapV3池子添加流动性
    交易hash: 0xabc123...
    池子地址: 0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640
    tickLower: -887220
    tickUpper: 887220
    
T1: 交易被打包进区块 18000000
    
T2: 节点检测到新区块
    new_block_headers_channel.send(Header { number: 18000000, ... })
    
T3: 调用 debug_traceBlock(18000000)
    返回 StateUpdate:
    {
        0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640: {
            storage: {
                0x0000...0000: 0x...new_slot0,      // sqrtPriceX96, tick
                0x0000...0004: 0x...new_liquidity,  // liquidity增加
                0xabcd...1234: 0x...new_bitmap,     // tickBitmap更新
                0xef01...5678: 0x...new_tick_data,  // tick.liquidityGross
            }
        }
    }
    
T4: 广播StateUpdate
    new_block_state_update_channel.send(BlockStateUpdate { ... })
    
T5: MarketState应用更新
    market_state.apply_geth_update(state_update)
    → LoomDB.storage[0x88e6...][0xabcd...1234] = 0x...new_bitmap
    
T6: 识别受影响的池子
    affected_pools = {
        PoolWrapper(0x88e6A0c2dDD26FEEb64F039a2c41296FcB3f5640): [
            SwapDirection { from: USDC, to: WETH },
            SwapDirection { from: WETH, to: USDC },
        ]
    }
    
T7: 触发套利搜索
    state_updates_broadcaster.send(StateUpdateEvent { ... })
    
T8: 套利计算
    calculate_arbitrage(db, swap_path, config)
    
T9: 读取tick数据
    tick_bitmap = UniswapV3DBReader::tick_bitmap(db, pool, word_pos)
    → 返回: 0x...new_bitmap ✅ (最新值!)
    
    tick_info = UniswapV3DBReader::ticks(db, pool, tick)
    → 返回: TickInfo { liquidity_gross: 新值, ... } ✅
    
T10: 完成套利计算
    profit = 0.05 ETH
    发送交易...
```

---

## 关键要点

### ✅ 自动同步
```rust
// 不需要手动调用
pool.reload_tick_data();  // ❌ 不需要

// 自动通过state_update同步
market_state.apply_geth_update(state_update);  // ✅ 自动更新所有storage
```

### ✅ 实时性
```rust
// 每个区块都会更新
loop {
    let block = wait_for_new_block().await;
    let state_update = debug_trace_block(block).await;
    market_state.apply_geth_update(state_update);  // ← 实时更新
}
```

### ✅ 准确性
```rust
// StateUpdate包含所有storage变化
GethStateUpdate {
    pool_address: AccountState {
        storage: {
            slot_0: new_value,      // ✅ slot0更新
            slot_4: new_liquidity,  // ✅ liquidity更新
            slot_6_bitmap: ...,     // ✅ tickBitmap更新
            slot_7_tick: ...,       // ✅ tick数据更新
        }
    }
}
```

### ✅ 效率
```rust
// 只更新变化的slot,不是整个池子
for (slot, value) in state_update.storage.iter() {
    db.insert_account_storage(address, slot, value);  // ← 增量更新
}

// 读取速度极快
let tick = UniswapV3DBReader::tick_bitmap(db, pool, word_pos);  // ~0.02ms
```

---

## 总结

这就是Loom如何保持本地DB与链上状态同步的完整机制:

1. **获取**: `debug_traceBlock` → StateUpdate
2. **应用**: `apply_geth_update` → LoomDB
3. **识别**: `get_affected_pools` → 受影响的池子
4. **计算**: `calculate_arbitrage` → 使用最新tick数据
5. **执行**: 发送套利交易

整个流程完全自动化,无需手动干预,保证了数据的实时性和准确性!
