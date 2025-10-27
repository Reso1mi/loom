# Loom 项目 RPC 传输方式分析

## 🎯 你的观察

> "我看项目中似乎并没有 http 类型的 rpc 都是把 wss 当作 http 使用的，这种是常规做法吗"

**你的观察非常准确！** 让我详细解释一下。

---

## 📊 实际代码分析

### 1. 配置文件中的定义

在 `config-example.toml` 中，确实定义了三种传输类型：

```toml
# TransportType 枚举定义
pub enum TransportType {
    Ws,      // WebSocket
    Http,    // HTTP
    Ipc,     // IPC (Unix Socket)
}
```

### 2. 实际实现代码

但在 `crates/core/topology/src/topology.rs` 中的实现：

```rust
let client = match config_params.transport {
    TransportType::Ipc => {
        info!("Starting IPC connection");
        let transport = IpcConnect::from(config_params.url);
        ClientBuilder::default().ipc(transport).await
    }
    _ => {  // 👈 注意这里！
        info!("Starting WS connection");
        let transport = WsConnect { url: config_params.url, auth: None, config: None };
        ClientBuilder::default().ws(transport).await
    }
};
```

**关键发现**：
- ✅ `TransportType::Ipc` 有专门的处理
- ❌ `TransportType::Http` 和 `TransportType::Ws` **都被当作 WebSocket 处理**！
- 使用 `_` 通配符，意味着除了 IPC 之外的所有类型都走 WebSocket

---

## 🤔 这是常规做法吗？

### 答案：**不是常规做法，但在 MEV 场景下是合理的选择**

让我解释原因：

### 1. WebSocket vs HTTP 的区别

| 特性 | WebSocket (wss://) | HTTP (https://) |
|------|-------------------|-----------------|
| **连接方式** | 持久连接 | 请求-响应 |
| **实时性** | 双向实时通信 | 单向请求 |
| **订阅支持** | ✅ 支持 `eth_subscribe` | ❌ 不支持订阅 |
| **Mempool 监听** | ✅ 实时推送 | ❌ 需要轮询 |
| **延迟** | 低（持久连接） | 高（每次建立连接） |
| **开销** | 低（连接复用） | 高（频繁握手） |

### 2. MEV 场景的需求

```rust
// MEV 机器人需要：
// 1. 实时监听新区块
provider.subscribe_blocks().await?;

// 2. 实时监听 pending 交易（mempool）
provider.subscribe_pending_transactions().await?;

// 3. 实时监听日志事件
provider.subscribe_logs(&filter).await?;
```

**这些功能都需要 WebSocket！HTTP 无法实现！**

### 3. 为什么配置中有 HTTP 选项？

可能的原因：

1. **历史遗留**：早期版本可能支持 HTTP
2. **未来扩展**：预留接口，但目前未实现
3. **文档示例**：展示不同的配置方式
4. **备用方案**：某些只读操作可以用 HTTP

---

## 🔍 深入分析：Alloy 库的支持

### Alloy ProviderBuilder 支持的方法

```rust
// 1. WebSocket 连接
let ws = WsConnect::new("wss://eth-mainnet.g.alchemy.com/v2/KEY");
let provider = ProviderBuilder::new().on_ws(ws).await?;

// 2. HTTP 连接
let provider = ProviderBuilder::new()
    .on_http("https://eth-mainnet.g.alchemy.com/v2/KEY".parse()?);

// 3. IPC 连接
let ipc = IpcConnect::from("/var/run/reth.ipc");
let provider = ProviderBuilder::new().on_ipc(ipc).await?;

// 4. 通用 Client（Loom 使用的方式）
let client = ClientBuilder::default().ws(transport).await?;
let provider = ProviderBuilder::new().on_client(client);
```

### Loom 为什么选择 `on_client` 而不是 `on_http`？

```rust
// Loom 的实现
let client = ClientBuilder::default().ws(transport).await?;
let provider = ProviderBuilder::new()
    .disable_recommended_fillers()  // 👈 关键：禁用自动填充
    .on_client(client);

// 如果用 on_http
let provider = ProviderBuilder::new()
    .on_http(url);  // 👈 无法禁用 fillers，无法订阅
```

**原因**：
1. 需要 `disable_recommended_fillers()` 来控制交易构建
2. 需要 WebSocket 的订阅功能
3. 需要统一的接口处理不同传输方式

---

## 📝 实际使用场景对比

### 场景 1：MEV 机器人（Loom 的场景）

```toml
# ✅ 正确：使用 WebSocket
[clients]
local = { 
    url = "ws://localhost:8546", 
    transport = "ws",  # 虽然写 ws，但实际也会用 ws
    node = "reth" 
}
```

**为什么必须用 WebSocket**：
- 需要实时监听 mempool
- 需要订阅新区块
- 需要低延迟（毫秒级竞争）

### 场景 2：只读查询（如区块浏览器）

```toml
# ✅ 可以使用 HTTP
[clients]
archive = { 
    url = "https://eth-mainnet.g.alchemy.com/v2/KEY", 
    transport = "http",  # 但 Loom 会当作 ws 处理！
    node = "geth" 
}
```

**HTTP 适用于**：
- 历史数据查询
- 偶尔的状态读取
- 不需要实时性的场景

### 场景 3：本地节点（最佳性能）

```toml
# ✅ 最佳：使用 IPC
[clients]
local = { 
    url = "/var/run/reth.ipc", 
    transport = "ipc",  # 真正的 IPC 实现
    db_path = "/data/reth/db",
    node = "reth" 
}
```

**IPC 优势**：
- 延迟最低（< 1ms）
- 无网络开销
- 支持订阅

---

## 🎯 常规做法对比

### 1. 标准 Web3 应用

```javascript
// 通常会区分 HTTP 和 WebSocket
const httpProvider = new Web3.providers.HttpProvider(
    'https://mainnet.infura.io/v3/KEY'
);

const wsProvider = new Web3.providers.WebsocketProvider(
    'wss://mainnet.infura.io/ws/v3/KEY'
);

// 根据需求选择
const web3 = new Web3(needsSubscription ? wsProvider : httpProvider);
```

### 2. Loom 的做法

```rust
// 统一使用 WebSocket（除了 IPC）
let client = match config_params.transport {
    TransportType::Ipc => /* IPC */,
    _ => /* WebSocket */,  // HTTP 也走这里！
};
```

**为什么这样做**：
1. **简化实现**：不需要维护两套代码
2. **功能需求**：MEV 必须用 WebSocket
3. **性能考虑**：WebSocket 在所有场景下都不差

---

## 💡 这种做法的优缺点

### ✅ 优点

1. **代码简洁**
   - 只需要维护 IPC 和 WebSocket 两种实现
   - 减少分支逻辑

2. **功能完整**
   - WebSocket 支持所有 HTTP 的功能
   - 额外支持订阅功能

3. **性能优秀**
   - 持久连接，减少握手开销
   - 适合高频调用

### ❌ 缺点

1. **配置误导**
   - 配置文件中有 `http` 选项，但实际不生效
   - 用户可能困惑

2. **资源占用**
   - WebSocket 持久连接占用资源
   - HTTP 可以按需连接

3. **兼容性**
   - 某些 RPC 提供商可能限制 WebSocket 连接数
   - HTTP 通常限制更宽松

---

## 🔧 建议的改进方案

### 方案 1：明确文档说明

```toml
# config-example.toml
[clients]
# 注意：虽然可以配置 transport = "http"，
# 但实际会使用 WebSocket 连接
# 因为 MEV 机器人需要实时订阅功能
local = { 
    url = "wss://eth-mainnet.g.alchemy.com/v2/KEY", 
    transport = "ws",  # 推荐明确使用 ws
    node = "geth" 
}
```

### 方案 2：实现真正的 HTTP 支持

```rust
let client = match config_params.transport {
    TransportType::Ipc => {
        let transport = IpcConnect::from(config_params.url);
        ClientBuilder::default().ipc(transport).await
    }
    TransportType::Http => {
        // 添加 HTTP 实现
        let url = config_params.url.parse()?;
        ClientBuilder::default().http(url)
    }
    TransportType::Ws => {
        let transport = WsConnect { url: config_params.url, auth: None, config: None };
        ClientBuilder::default().ws(transport).await
    }
};
```

### 方案 3：移除 HTTP 选项

```rust
pub enum TransportType {
    Ws,   // WebSocket - 支持订阅
    Ipc,  // IPC - 本地最快
    // 移除 Http，因为不支持
}
```

---

## 📊 实际使用建议

### 对于 Loom 用户

```toml
# ✅ 推荐配置
[clients]
# 本地节点 - 使用 IPC（最快）
local = { 
    url = "/var/run/reth.ipc", 
    transport = "ipc", 
    db_path = "/data/reth/db",
    node = "reth" 
}

# 远程节点 - 使用 WebSocket
remote = { 
    url = "wss://eth-mainnet.g.alchemy.com/v2/KEY", 
    transport = "ws",  # 不要用 http！
    node = "geth" 
}
```

### 对于其他项目

```rust
// 如果不需要订阅功能，HTTP 是可以的
let provider = ProviderBuilder::new()
    .on_http("https://eth-mainnet.g.alchemy.com/v2/KEY".parse()?);

// 如果需要订阅，必须用 WebSocket
let ws = WsConnect::new("wss://eth-mainnet.g.alchemy.com/v2/KEY");
let provider = ProviderBuilder::new().on_ws(ws).await?;
```

---

## 🎯 总结

### 你的观察是正确的！

1. **Loom 确实把 HTTP 当作 WebSocket 处理**
   - 代码中 `_` 通配符包含了 `TransportType::Http`
   - 实际都调用 `ClientBuilder::default().ws()`

2. **这不是常规做法**
   - 标准做法是区分 HTTP 和 WebSocket
   - 根据需求选择合适的传输方式

3. **但在 MEV 场景下是合理的**
   - MEV 机器人必须用 WebSocket（订阅功能）
   - 简化实现，减少维护成本
   - WebSocket 性能不差于 HTTP

4. **建议**
   - 配置文件中明确使用 `transport = "ws"`
   - 不要依赖 `transport = "http"`（实际不生效）
   - 本地节点优先使用 IPC

### 关键要点

```
MEV 机器人的核心需求：
1. 实时监听 mempool → 必须 WebSocket
2. 实时监听新区块 → 必须 WebSocket  
3. 低延迟响应 → WebSocket 或 IPC

因此，Loom 统一使用 WebSocket 是合理的设计选择！
```

---

## 📚 相关资源

- [Alloy Provider 文档](https://alloy.rs/)
- [Ethereum JSON-RPC 规范](https://ethereum.org/en/developers/docs/apis/json-rpc/)
- [WebSocket vs HTTP 对比](https://www.pubnub.com/blog/websockets-vs-rest-api-understanding-the-difference/)
