use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::Provider;
use alloy::rpc::types::{BlockNumberOrTag, Log};
use alloy::sol;
use alloy::sol_types::{SolEvent, SolInterface};
use async_trait::async_trait;
use eyre::{eyre, Result};
use lazy_static::lazy_static;
use tracing::debug;

use crate::pool_loader_trait::PoolLoader;
use crate::types::{PoolClass, PoolData, PoolId, PoolProtocol, UniswapV2PoolData};

// 定义 UniswapV2 合约接口
sol! {
    #[sol(rpc)]
    interface IUniswapV2Pair {
        function token0() external view returns (address);
        function token1() external view returns (address);
        function factory() external view returns (address);
        function getReserves() external view returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
        
        event Swap(address indexed sender, uint amount0In, uint amount1In, uint amount0Out, uint amount1Out, address indexed to);
        event Sync(uint112 reserve0, uint112 reserve1);
        event Mint(address indexed sender, uint amount0, uint amount1);
        event Burn(address indexed sender, uint amount0, uint amount1, address indexed to);
    }
}

lazy_static! {
    static ref U112_MASK: U256 = (U256::from(1) << 112) - U256::from(1);
    
    // 已知的 UniswapV2 Factory 地址
    static ref UNISWAP_V2_FACTORY: Address = 
        "0x5C69bEe701ef814a2B6a3EDD4B1652CB9cc5aA6f".parse().unwrap();
    static ref SUSHISWAP_FACTORY: Address = 
        "0xC0AEe478e3658e2610c5F7A4A2E1777cE9e4f2Ac".parse().unwrap();
}

/// UniswapV2 池子加载器
pub struct UniswapV2Loader;

impl UniswapV2Loader {
    pub fn new() -> Self {
        Self
    }
    
    /// 根据 factory 地址判断协议类型
    fn get_protocol_by_factory(factory: Address) -> PoolProtocol {
        if factory == *UNISWAP_V2_FACTORY {
            PoolProtocol::UniswapV2
        } else if factory == *SUSHISWAP_FACTORY {
            PoolProtocol::Sushiswap
        } else {
            PoolProtocol::UniswapV2Like
        }
    }
    
    /// 根据协议类型获取手续费
    fn get_fee_by_protocol(protocol: PoolProtocol) -> U256 {
        match protocol {
            PoolProtocol::UniswapV2 | PoolProtocol::Sushiswap => U256::from(9970),
            _ => U256::from(9970),
        }
    }
    
    /// 从 storage 值解析 reserves
    fn storage_to_reserves(value: U256) -> (U256, U256) {
        let reserve0 = (value >> 0) & *U112_MASK;
        let reserve1 = (value >> 112) & *U112_MASK;
        (reserve0, reserve1)
    }
}

#[async_trait]
impl PoolLoader for UniswapV2Loader {
    fn identify_pool_from_log(&self, log: &Log) -> Option<(PoolId, PoolClass)> {
        // 检查是否是 UniswapV2 相关事件
        // Swap, Sync, Mint, Burn 事件都表明这是一个 UniswapV2 池子
        
        if log.topics().is_empty() {
            return None;
        }
        
        let topic0 = log.topics()[0];
        
        // 检查事件签名
        if topic0 == IUniswapV2Pair::Swap::SIGNATURE_HASH ||
           topic0 == IUniswapV2Pair::Sync::SIGNATURE_HASH ||
           topic0 == IUniswapV2Pair::Mint::SIGNATURE_HASH ||
           topic0 == IUniswapV2Pair::Burn::SIGNATURE_HASH {
            Some((PoolId(log.address()), PoolClass::UniswapV2))
        } else {
            None
        }
    }
    
    async fn fetch_pool_data<P>(&self, provider: &P, pool_id: PoolId) -> Result<PoolData>
    where
        P: Provider + Clone + 'static,
    {
        let address = pool_id.address();
        
        // 创建合约实例
        let pair = IUniswapV2Pair::new(address, provider.clone());
        
        // 并行调用合约方法获取基础信息
        let (token0_result, token1_result, factory_result, reserves_result) = tokio::join!(
            pair.token0(),
            pair.token1(),
            pair.factory(),
            pair.getReserves()
        );
        
        let token0 = token0_result?._0;
        let token1 = token1_result?._0;
        let factory = factory_result?._0;
        let reserves = reserves_result?;
        
        // 尝试获取 reserves 的 storage slot（优化：后续可以直接读 storage）
        let storage_value = provider
            .get_storage_at(address, U256::from(8))
            .block_id(BlockNumberOrTag::Latest.into())
            .await?;
        
        let storage_reserves = Self::storage_to_reserves(storage_value);
        
        // 验证 storage slot 是否正确
        let reserves_slot = if storage_reserves.0 == U256::from(reserves.reserve0) &&
                               storage_reserves.1 == U256::from(reserves.reserve1) {
            Some(U256::from(8))
        } else {
            debug!(
                "Storage slot mismatch for pool {}: storage=({}, {}), contract=({}, {})",
                address, storage_reserves.0, storage_reserves.1, 
                reserves.reserve0, reserves.reserve1
            );
            None
        };
        
        let protocol = Self::get_protocol_by_factory(factory);
        let fee = Self::get_fee_by_protocol(protocol);
        
        Ok(PoolData::UniswapV2(UniswapV2PoolData {
            address,
            token0,
            token1,
            factory,
            protocol,
            fee,
            reserve0: U256::from(reserves.reserve0),
            reserve1: U256::from(reserves.reserve1),
            reserves_slot,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_storage_to_reserves() {
        // 测试 reserves 解析
        let value = U256::from(100u64) | (U256::from(200u64) << 112);
        let (r0, r1) = UniswapV2Loader::storage_to_reserves(value);
        assert_eq!(r0, U256::from(100u64));
        assert_eq!(r1, U256::from(200u64));
    }
}
