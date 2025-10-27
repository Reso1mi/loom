# Loom Pool Loader - 简化架构版本

## 📋 项目说明

这是 Loom 项目历史池子加载功能的**简化架构版本**。

### 🎯 设计目标

1. **保留核心架构**：Actor 系统、消息传递、PoolLoaders 容器
2. **降低抽象程度**：减少泛型参数、简化 trait 定义、去除宏
3. **提高可读性**：清晰的代码结构、详细的注释
4. **保持性能**：异步并发、Semaphore 控制

### 📊 与原始版本对比

| 方面 | 原始版本 | 简化版本 | 改进 |
|------|---------|---------|------|
| **泛型参数** | 3-5 个 (P, PL, N, LDT) | 1 个 (P) | ✅ 减少 60-80% |
| **异步实现** | `Pin<Box<dyn Future>>` | `async fn` (async-trait) | ✅ 更简洁 |
| **代码生成** | 宏 `pool_loader!()` | 直接定义 | ✅ 更清晰 |
| **调用层级** | 10+ 层 | 6-7 层 | ✅ 减少 30% |
| **文件数量** | 15+ 文件 | 10 文件 | ✅ 更集中 |
| **学习曲线** | 陡峭（数周） | 平缓（数小时） | ✅ 大幅降低 |

## 🏗️ 架构设计

### 核心组件

```
┌─────────────────────────────────────────────────────────┐
│                    Main Application                      │
└─────────────────────────────────────────────────────────┘
                            │
                ┌───────────┴───────────┐
                │                       │
        ┌───────▼────────┐      ┌──────▼──────┐
        │ HistoryLoader  │      │ PoolLoader  │
        │     Actor      │      │    Actor    │
        └───────┬────────┘      └──────┬──────┘
                │                      │
                │   Message Channel    │
                └──────────┬───────────┘
                           │
                    ┌──────▼──────┐
                    │ PoolLoaders │
                    │  Container  │
                    └──────┬──────┘
                           │
            ┌──────────────┼──────────────┐
            │              │              │
    ┌───────▼────┐  ┌──────▼─────┐  ┌────▼─────┐
    │ UniswapV2  │  │ UniswapV3  │  │  Curve   │
    │   Loader   │  │   Loader   │  │  Loader  │
    └────────────┘  └────────────┘  └──────────┘
```

### 数据流

```
1. HistoryLoaderActor
   ↓ 扫描历史区块
   ↓ 获取日志
   ↓ 识别池子类型
   ↓ 发送 Message::FetchPools
   
2. PoolLoaderActor
   ↓ 接收消息
   ↓ 并发加载池子数据
   ↓ 发送 Message::PoolLoaded
   
3. Collector
   ↓ 收集结果
   ↓ 保存到文件
```

## 📁 文件结构

```
src/
├── main.rs                      # 主程序入口
├── types.rs                     # 类型定义
├── messages.rs                  # 消息类型
├── pool_loader_trait.rs         # PoolLoader trait
├── pool_loaders.rs              # PoolLoaders 容器
├── uniswap_v2_loader.rs         # UniswapV2 加载器
├── uniswap_v3_loader.rs         # UniswapV3 加载器
└── actors/
    ├── mod.rs                   # Actor trait
    ├── history_loader_actor.rs  # 历史加载 Actor
    └── pool_loader_actor.rs     # 池子加载 Actor
```

## 🔑 核心简化点

### 1. 减少泛型参数

**原始版本**：
```rust
pub struct HistoryPoolLoaderOneShotActor<P, PL, N>
where
    N: Network,
    P: Provider<N> + Send + Sync + Clone + 'static,
    PL: Provider<N> + Send + Sync + Clone + 'static,
{
    client: P,
    pool_loaders: Arc<PoolLoaders<PL, N>>,
    _n: PhantomData<N>,
}
```

**简化版本**：
```rust
pub struct HistoryPoolLoaderActor<P>
where
    P: Provider + Clone + Send + Sync + 'static,
{
    provider: P,
    pool_loaders: Arc<PoolLoaders>,
    config: LoaderConfig,
    message_tx: broadcast::Sender<Message>,
}
```

### 2. 使用 async-trait

**原始版本**：
```rust
fn fetch_pool_by_id<'a>(&'a self, pool_id: PoolId) 
    -> Pin<Box<dyn Future<Output = Result<PoolWrapper>> + Send + 'a>>
{
    Box::pin(async move {
        // ...
    })
}
```

**简化版本**：
```rust
#[async_trait]
pub trait PoolLoader: Send + Sync {
    async fn fetch_pool_data<P>(&self, provider: &P, pool_id: PoolId) 
        -> Result<PoolData>
    where
        P: Provider + Clone + 'static;
}
```

### 3. 去除宏生成

**原始版本**：
```rust
pool_loader!(UniswapV2PoolLoader);  // 宏生成结构体
```

**简化版本**：
```rust
pub struct UniswapV2Loader;  // 直接定义

impl UniswapV2Loader {
    pub fn new() -> Self {
        Self
    }
}
```

### 4. 简化消息类型

**原始版本**：
```rust
pub enum LoomTask {
    FetchAndAddPools(Vec<(PoolId, PoolClass)>),
    // ... 更多复杂类型
}
```

**简化版本**：
```rust
pub enum Message {
    FetchPools(Vec<(PoolId, PoolClass)>),
    PoolLoaded(PoolData),
    Stop,
}
```

## 🚀 使用方法

### 1. 设置环境变量

```bash
export RPC_URL="wss://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY"
```

### 2. 运行

```bash
cd loom-rewrite/step1-pool-loader-v2
cargo run --release
```

### 3. 查看结果

```bash
# 查看加载的池子数量
cat pools.json | jq length

# 查看第一个池子
cat pools.json | jq '.[0]'

# 统计池子类型
cat pools.json | jq 'group_by(.UniswapV2 != null) | map({type: (if .[0].UniswapV2 then "V2" else "V3" end), count: length})'
```

## 📖 代码阅读顺序

### 入门（1-2 小时）

1. **types.rs** - 理解数据结构
   - `PoolId`, `PoolClass`, `PoolData`
   - 简单的枚举和结构体

2. **messages.rs** - 理解消息类型
   - Actor 之间如何通信

3. **pool_loader_trait.rs** - 理解核心接口
   - `PoolLoader` trait 定义
   - `async-trait` 的使用

### 进阶（2-3 小时）

4. **uniswap_v2_loader.rs** - 理解具体实现
   - 如何识别 UniswapV2 池子
   - 如何获取池子数据
   - `sol!` 宏的使用

5. **pool_loaders.rs** - 理解容器设计
   - 如何管理多个加载器
   - trait object 的使用

6. **actors/mod.rs** - 理解 Actor trait
   - Actor 的基本接口

### 深入（3-5 小时）

7. **actors/history_loader_actor.rs** - 理解历史扫描
   - 如何扫描区块
   - 如何处理日志
   - 如何发送消息

8. **actors/pool_loader_actor.rs** - 理解并发加载
   - 如何接收消息
   - 如何并发控制（Semaphore）
   - 如何加载池子数据

9. **main.rs** - 理解整体流程
   - 如何初始化组件
   - 如何启动 Actor
   - 如何收集结果

## 🎓 学习要点

### 1. Actor 模式

- **定义**：每个 Actor 是独立的执行单元
- **通信**：通过消息传递（broadcast channel）
- **优点**：解耦、并发、容错

### 2. 消息传递

```rust
// 发送消息
message_tx.send(Message::FetchPools(pools))?;

// 接收消息
match message_rx.recv().await {
    Ok(Message::FetchPools(pools)) => { /* 处理 */ }
    // ...
}
```

### 3. 并发控制

```rust
// 使用 Semaphore 限制并发数
let semaphore = Arc::new(Semaphore::new(20));

// 获取许可
let _permit = semaphore.acquire().await?;
// 执行任务
// _permit 自动释放
```

### 4. Trait Object

```rust
// 存储不同类型的 PoolLoader
HashMap<PoolClass, Arc<dyn PoolLoader>>

// 动态调用
loader.fetch_pool_data(provider, pool_id).await?
```

## 🔍 与原始版本的关键区别

### 保留的设计

✅ Actor 系统架构  
✅ 消息传递机制  
✅ PoolLoaders 容器  
✅ PoolLoader trait  
✅ 并发控制（Semaphore）  
✅ 异步编程模型  

### 简化的部分

🔧 泛型参数：从 3-5 个减少到 1 个  
🔧 异步方法：使用 `async-trait` 代替 `Pin<Box<dyn Future>>`  
🔧 代码生成：去除宏，直接定义结构体  
🔧 类型系统：使用具体类型代替过度抽象  
🔧 调用链：减少中间层，缩短调用路径  

## 💡 关键洞察

1. **Actor 模式的价值**
   - 清晰的职责分离
   - 天然的并发支持
   - 易于测试和扩展

2. **适度抽象的重要性**
   - 过度抽象增加复杂度
   - 具体类型提高可读性
   - 平衡灵活性和简洁性

3. **async-trait 的优势**
   - 简化异步 trait 方法
   - 提高代码可读性
   - 轻微的性能开销可接受

## 🎯 下一步

1. **添加更多池子类型**
   - Curve
   - Balancer
   - 自定义池子

2. **优化性能**
   - 批量 RPC 调用
   - 缓存机制
   - 数据库存储

3. **添加实时监控**
   - WebSocket 订阅
   - 实时事件处理

4. **集成到完整系统**
   - 套利路径搜索
   - 交易执行
   - 利润计算

## 📚 参考资料

- [Alloy 文档](https://alloy.rs/)
- [Tokio 文档](https://tokio.rs/)
- [async-trait](https://docs.rs/async-trait/)
- [Actor 模式](https://en.wikipedia.org/wiki/Actor_model)
