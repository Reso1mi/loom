use alloy::primitives::{Address, U256};
use serde::{Deserialize, Serialize};

/// 池子 ID - 简化版本，只使用 Address
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoolId(pub Address);

impl PoolId {
    pub fn address(&self) -> Address {
        self.0
    }
}

/// 池子类型分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoolClass {
    UniswapV2,
    UniswapV3,
    Curve,
    Maverick,
}

/// 池子协议类型（更细分）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolProtocol {
    // UniswapV2 系列
    UniswapV2,
    Sushiswap,
    UniswapV2Like,
    
    // UniswapV3 系列
    UniswapV3,
    SushiswapV3,
    PancakeV3,
    UniswapV3Like,
    
    // 其他
    Curve,
    Maverick,
}

/// UniswapV2 池子数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniswapV2PoolData {
    pub address: Address,
    pub token0: Address,
    pub token1: Address,
    pub factory: Address,
    pub protocol: PoolProtocol,
    pub fee: U256,
    pub reserve0: U256,
    pub reserve1: U256,
    /// Storage slot for reserves (优化：直接读取 storage)
    pub reserves_slot: Option<U256>,
}

/// UniswapV3 池子数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniswapV3PoolData {
    pub address: Address,
    pub token0: Address,
    pub token1: Address,
    pub factory: Address,
    pub protocol: PoolProtocol,
    pub fee: u32,
    pub liquidity: u128,
    pub sqrt_price_x96: U256,
    pub tick: i32,
}

/// 统一的池子数据包装
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PoolData {
    UniswapV2(UniswapV2PoolData),
    UniswapV3(UniswapV3PoolData),
}

impl PoolData {
    pub fn address(&self) -> Address {
        match self {
            PoolData::UniswapV2(pool) => pool.address,
            PoolData::UniswapV3(pool) => pool.address,
        }
    }
    
    pub fn pool_class(&self) -> PoolClass {
        match self {
            PoolData::UniswapV2(_) => PoolClass::UniswapV2,
            PoolData::UniswapV3(_) => PoolClass::UniswapV3,
        }
    }
    
    pub fn tokens(&self) -> (Address, Address) {
        match self {
            PoolData::UniswapV2(pool) => (pool.token0, pool.token1),
            PoolData::UniswapV3(pool) => (pool.token0, pool.token1),
        }
    }
}

/// 加载配置
#[derive(Debug, Clone)]
pub struct LoaderConfig {
    /// 最大并发任务数
    pub max_concurrent_tasks: usize,
    /// 起始区块
    pub start_block: u64,
    /// 每次扫描的区块数
    pub block_batch_size: u64,
    /// 扫描的批次数
    pub num_batches: usize,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 20,
            start_block: 0,  // 0 表示从最新区块开始
            block_batch_size: 5,
            num_batches: 10000,
        }
    }
}
