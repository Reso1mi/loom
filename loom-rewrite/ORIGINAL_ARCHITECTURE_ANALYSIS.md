# Loom 原始架构深度分析

## 📋 核心架构概览

### 1. Actor 系统设计

原始项目使用了完整的 Actor 模型：

```rust
// Actor trait 定义
pub trait Actor {
    fn start(&self) -> ActorResult;  // 返回 Vec<JoinHandle<WorkerResult>>
    fn name(&self) -> &'static str;
}

// 三种角色
pub trait Producer {  // 生产者：发送消息
    fn produce(&mut self, channel: Broadcaster<T>) -> Self;
}

pub trait Consumer {  // 消费者：接收消息
    fn consume(&mut self, channel: Broadcaster<T>) -> Self;
}

pub trait Accessor {  // 访问者：访问共享状态
    fn access(&mut self, state: SharedState<T>) -> Self;
}
```

### 2. 历史池子加载流程

#### 完整调用链

```
1. HistoryPoolLoaderOneShotActor::start()
   ↓
2. history_pool_loader_one_shot_worker()
   - 循环获取历史区块
   - 每次处理 5 个区块
   ↓
3. client.get_logs(&filter)
   - 获取区块日志
   ↓
4. process_log_entries(logs, pool_loaders, tasks_tx)
   - 解析日志，识别池子
   ↓
5. pool_loaders.determine_pool_class(&log_entry)
   - 遍历所有 PoolLoader
   - 调用 get_pool_class_by_log()
   ↓
6. tasks_tx.send(LoomTask::FetchAndAddPools(pool_to_fetch))
   - 发送消息到 tasks_tx 通道
   ↓
7. PoolLoaderActor 接收消息
   ↓
8. pool_loader_worker() 处理消息
   - 使用 Semaphore 控制并发
   ↓
9. fetch_and_add_pool_by_pool_id()
   ↓
10. pool_loaders.load_pool_without_provider(pool_id, pool_class)
    ↓
11. pool_loader.fetch_pool_by_id(pool_id)
    - 调用具体的 PoolLoader 实现
    ↓
12. UniswapV2Pool::fetch_pool_data(provider, address)
    - 实际的 RPC 调用
```

### 3. 关键组件分析

#### 3.1 PoolLoaders 容器

```rust
pub struct PoolLoaders<P, N, LDT> {
    provider: Option<P>,
    config: Option<PoolsLoadingConfig>,
    pub map: HashMap<PoolClass, Arc<dyn PoolLoader<P, N, LDT>>>,
}
```

**作用**：
- 管理多种池子类型的加载器（UniswapV2, UniswapV3, Curve, Maverick）
- 提供统一的接口来识别和加载池子
- 使用 trait object (`Arc<dyn PoolLoader>`) 实现多态

#### 3.2 PoolLoader Trait

```rust
pub trait PoolLoader<P, N, LDT> {
    // 从日志识别池子类型
    fn get_pool_class_by_log(&self, log_entry: &LDT::Log) 
        -> Option<(PoolId<LDT>, PoolClass)>;
    
    // 从 provider 加载池子数据
    fn fetch_pool_by_id<'a>(&'a self, pool_id: PoolId<LDT>) 
        -> Pin<Box<dyn Future<Output = Result<PoolWrapper<LDT>>> + Send + 'a>>;
    
    // 从 EVM 状态加载池子数据
    fn fetch_pool_by_id_from_evm(&self, pool_id: PoolId<LDT>, 
        db: &dyn DatabaseRef, env: Env) -> Result<PoolWrapper<LDT>>;
    
    // 检查合约代码
    fn is_code(&self, code: &Bytes) -> bool;
    
    // 协议级别的加载器
    fn protocol_loader(&self) 
        -> Result<Pin<Box<dyn Stream<Item = (PoolId, PoolClass)> + Send>>>;
}
```

**复杂点**：
- 使用 `Pin<Box<dyn Future>>` 实现异步 trait 方法
- 生命周期标注 `'a` 确保引用有效性

#### 3.3 具体实现：UniswapV2PoolLoader

```rust
// 宏生成基础结构
pool_loader!(UniswapV2PoolLoader);

// 展开后：
pub struct UniswapV2PoolLoader<P, N, LDT> {
    provider: Option<P>,
    phantom_data: PhantomData<(P, N, LDT)>,
}

// 实现 PoolLoader trait
impl<P> PoolLoader<P, Ethereum, LoomDataTypesEthereum> 
    for UniswapV2PoolLoader<P, Ethereum, LoomDataTypesEthereum>
where
    P: Provider<Ethereum> + Clone + 'static,
{
    fn get_pool_class_by_log(&self, log_entry: &Log) 
        -> Option<(PoolId, PoolClass)> 
    {
        // 解析日志，识别 UniswapV2 事件
        match IUniswapV2PairEvents::decode_log(&log_entry, false) {
            Ok(event) => match event.data {
                IUniswapV2PairEvents::Swap(_) |
                IUniswapV2PairEvents::Mint(_) |
                IUniswapV2PairEvents::Burn(_) |
                IUniswapV2PairEvents::Sync(_) => 
                    Some((PoolId::Address(log_entry.address), PoolClass::UniswapV2)),
                _ => None,
            },
            Err(_) => None,
        }
    }
    
    fn fetch_pool_by_id<'a>(&'a self, pool_id: PoolId) 
        -> Pin<Box<dyn Future<Output = Result<PoolWrapper>> + Send + 'a>> 
    {
        Box::pin(async move {
            if let Some(provider) = self.provider.clone() {
                self.fetch_pool_by_id_from_provider(pool_id, provider).await
            } else {
                Err(eyre!("NO_PROVIDER"))
            }
        })
    }
    
    fn fetch_pool_by_id_from_provider<'a>(&'a self, pool_id: PoolId, provider: P) 
        -> Pin<Box<dyn Future<Output = Result<PoolWrapper>> + Send + 'a>> 
    {
        Box::pin(async move {
            let pool_address = pool_id.address()?;
            let factory_address = fetch_uni2_factory(provider.clone(), pool_address).await?;
            
            // 根据 factory 判断协议类型
            match get_protocol_by_factory(factory_address) {
                PoolProtocol::NomiswapStable | ... => 
                    Err(eyre!("POOL_PROTOCOL_NOT_SUPPORTED")),
                _ => Ok(PoolWrapper::new(Arc::new(
                    UniswapV2Pool::fetch_pool_data(provider, pool_address).await?
                ))),
            }
        })
    }
}
```

#### 3.4 UniswapV2Pool 数据获取

```rust
pub async fn fetch_pool_data<N, P>(client: P, address: Address) -> Result<Self>
where
    N: Network,
    P: Provider<N> + Send + Sync + Clone + 'static,
{
    let uni2_pool = IUniswapV2Pair::IUniswapV2PairInstance::new(address, client.clone());
    
    // 并行调用合约方法
    let token0: Address = uni2_pool.token0().call().await?._0;
    let token1: Address = uni2_pool.token1().call().await?._0;
    let factory: Address = uni2_pool.factory().call().await?._0;
    let reserves = uni2_pool.getReserves().call().await?;
    
    // 获取 storage slot（优化）
    let storage_reserves_cell = client
        .get_storage_at(address, U256::from(8))
        .block_id(BlockNumberOrTag::Latest.into())
        .await?;
    
    let storage_reserves = Self::storage_to_reserves(storage_reserves_cell);
    
    // 验证 storage slot 是否正确
    let reserves_cell: Option<U256> = 
        if storage_reserves.0 == U256::from(reserves.reserve0) && 
           storage_reserves.1 == U256::from(reserves.reserve1) {
            Some(U256::from(8))
        } else {
            None
        };
    
    let protocol = UniswapV2Pool::get_uni2_protocol_by_factory(factory);
    let fee = Self::get_fee_by_protocol(protocol);
    
    Ok(UniswapV2Pool {
        address,
        token0,
        token1,
        factory,
        protocol,
        fee,
        reserves_cell,
        liquidity0: U256::from(reserves.reserve0),
        liquidity1: U256::from(reserves.reserve1),
        encoder: UniswapV2PoolAbiEncoder {},
    })
}
```

### 4. 消息传递机制

#### 4.1 Broadcaster 通道

```rust
pub type Broadcaster<T> = Arc<tokio::sync::broadcast::Sender<T>>;

// 使用方式
tasks_tx.send(LoomTask::FetchAndAddPools(pool_to_fetch))?;

// 接收方
subscribe!(tasks_rx);  // 宏展开为：let mut tasks_rx = tasks_rx.subscribe();
loop {
    if let Ok(task) = tasks_rx.recv().await {
        match task {
            LoomTask::FetchAndAddPools(pools) => {
                // 处理池子加载任务
            }
        }
    }
}
```

#### 4.2 LoomTask 消息类型

```rust
pub enum LoomTask {
    FetchAndAddPools(Vec<(PoolId, PoolClass)>),
}
```

### 5. 并发控制

```rust
// 使用 Semaphore 限制并发数
let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_TASKS));

for (pool_id, pool_class) in pools {
    let sema_clone = semaphore.clone();
    
    tokio::task::spawn(async move {
        // 获取许可
        match sema_clone.acquire().await {
            Ok(permit) => {
                // 执行池子加载
                match fetch_and_add_pool_by_pool_id(...).await {
                    Ok((pool_id, swap_path_idx_vec)) => {
                        info!("Pool loaded successfully");
                        market_events_tx.send(MarketEvents::NewPoolLoaded { 
                            pool_id, 
                            swap_path_idx_vec 
                        })?;
                    }
                    Err(error) => {
                        error!("failed fetch_and_add_pool_by_address");
                    }
                }
                drop(permit);  // 释放许可
            }
            Err(error) => {
                error!("failed acquire semaphore");
            }
        }
    });
}
```

### 6. 状态管理

#### 6.1 SharedState

```rust
pub type SharedState<T> = Arc<RwLock<T>>;

// 使用方式
let market_write_guard = market.write().await;
market_write_guard.add_pool(pool_wrapped)?;
drop(market_write_guard);
```

#### 6.2 Market 状态

```rust
// 添加池子到市场
let mut market_write_guard = market.write().await;
market_write_guard.add_pool(pool_wrapped)?;

// 构建交易路径
let swap_paths = market_write_guard.build_swap_path_vec(&directions_tree)?;
let swap_paths_added = market_write_guard.add_paths(swap_paths);

// 添加池子管理器单元
for (pool_manager_address, cells_vec) in pool_manager_cells {
    for cell in cells_vec {
        market_write_guard.add_pool_manager_cell(
            pool_manager_address, 
            pool_id, 
            cell
        );
    }
}
```

## 🎯 架构优缺点分析

### ✅ 优点

1. **模块化设计**
   - Actor 系统清晰分离关注点
   - 每个 Actor 独立运行，易于测试

2. **可扩展性**
   - 添加新的池子类型只需实现 PoolLoader trait
   - 使用 trait object 实现多态

3. **并发性能**
   - 使用 tokio 异步运行时
   - Semaphore 控制并发数
   - 消息传递避免锁竞争

4. **类型安全**
   - 泛型确保编译时类型检查
   - PhantomData 标记类型关系

### ❌ 缺点（过度抽象）

1. **复杂的泛型嵌套**
   ```rust
   HistoryPoolLoaderOneShotActor<P, PL, N>
   where
       N: Network,
       P: Provider<N> + Send + Sync + Clone + 'static,
       PL: Provider<N> + Send + Sync + Clone + 'static,
   ```
   - 三个泛型参数
   - 复杂的 trait bounds
   - PhantomData 标记

2. **深层调用链**
   - 10+ 层函数调用
   - 跨多个文件和模块
   - 难以追踪数据流

3. **Pin<Box<dyn Future>> 抽象**
   ```rust
   fn fetch_pool_by_id<'a>(&'a self, pool_id: PoolId) 
       -> Pin<Box<dyn Future<Output = Result<PoolWrapper>> + Send + 'a>>
   ```
   - 手动实现异步 trait 方法
   - 生命周期复杂

4. **宏生成代码**
   ```rust
   pool_loader!(UniswapV2PoolLoader);
   ```
   - 隐藏实现细节
   - IDE 支持差
   - 调试困难

5. **消息传递开销**
   - 每个池子都要经过消息队列
   - 序列化/反序列化开销
   - 增加延迟

## 🔧 简化方向

### 保留的核心设计

1. **Actor 模式** - 保持，但简化 trait 定义
2. **PoolLoaders 容器** - 保持，但去除部分泛型
3. **消息传递** - 保持，用于解耦
4. **并发控制** - 保持 Semaphore

### 需要简化的部分

1. **减少泛型参数**
   - 固定 Network = Ethereum
   - 固定 LDT = LoomDataTypesEthereum
   - 只保留 Provider 泛型

2. **简化 PoolLoader trait**
   - 使用 async fn（需要 async-trait）
   - 去除 Pin<Box<dyn Future>>

3. **去除宏生成**
   - 直接定义结构体
   - 提高代码可读性

4. **减少调用层级**
   - 合并部分中间函数
   - 直接调用核心逻辑

5. **简化类型别名**
   - 使用具体类型而非泛型
   - 减少 PhantomData

## 📊 对比总结

| 方面 | 原始设计 | 简化方向 |
|------|---------|---------|
| **泛型参数** | 3-5 个 | 1-2 个 |
| **调用层级** | 10+ 层 | 5-7 层 |
| **异步实现** | Pin<Box<dyn Future>> | async fn |
| **代码生成** | 宏 | 直接定义 |
| **类型系统** | 高度抽象 | 具体类型 |
| **学习曲线** | 陡峭 | 平缓 |
| **可维护性** | 困难 | 中等 |
| **性能** | 优秀 | 优秀 |

## 🎯 下一步

基于这个分析，我将创建一个**保持核心架构但降低抽象程度**的版本：

1. 保留 Actor 系统和消息传递
2. 保留 PoolLoaders 和 PoolLoader trait
3. 简化泛型参数
4. 使用 async-trait 简化异步方法
5. 去除宏，直接定义结构体
6. 减少中间层，缩短调用链
7. 添加详细注释，提高可读性
