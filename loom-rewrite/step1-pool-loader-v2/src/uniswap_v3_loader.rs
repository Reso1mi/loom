use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use alloy::rpc::types::Log;
use alloy::sol;
use alloy::sol_types::SolEvent;
use async_trait::async_trait;
use eyre::Result;
use lazy_static::lazy_static;

use crate::pool_loader_trait::PoolLoader;
use crate::types::{PoolClass, PoolData, PoolId, PoolProtocol, UniswapV3PoolData};

// 定义 UniswapV3 合约接口
sol! {
    #[sol(rpc)]
    interface IUniswapV3Pool {
        function token0() external view returns (address);
        function token1() external view returns (address);
        function factory() external view returns (address);
        function fee() external view returns (uint24);
        function liquidity() external view returns (uint128);
        function slot0() external view returns (
            uint160 sqrtPriceX96,
            int24 tick,
            uint16 observationIndex,
            uint16 observationCardinality,
            uint16 observationCardinalityNext,
            uint8 feeProtocol,
            bool unlocked
        );
        
        event Swap(
            address indexed sender,
            address indexed recipient,
            int256 amount0,
            int256 amount1,
            uint160 sqrtPriceX96,
            uint128 liquidity,
            int24 tick
        );
        event Mint(
            address sender,
            address indexed owner,
            int24 indexed tickLower,
            int24 indexed tickUpper,
            uint128 amount,
            uint256 amount0,
            uint256 amount1
        );
        event Burn(
            address indexed owner,
            int24 indexed tickLower,
            int24 indexed tickUpper,
            uint128 amount,
            uint256 amount0,
            uint256 amount1
        );
        event Initialize(uint160 sqrtPriceX96, int24 tick);
    }
}

lazy_static! {
    // 已知的 UniswapV3 Factory 地址
    static ref UNISWAP_V3_FACTORY: Address = 
        "0x1F98431c8aD98523631AE4a59f267346ea31F984".parse().unwrap();
    static ref SUSHISWAP_V3_FACTORY: Address = 
        "0xbACEB8eC6b9355Dfc0269C18bac9d6E2Bdc29C4F".parse().unwrap();
    static ref PANCAKE_V3_FACTORY: Address = 
        "0x0BFbCF9fa4f9C56B0F40a671Ad40E0805A091865".parse().unwrap();
}

/// UniswapV3 池子加载器
pub struct UniswapV3Loader;

impl UniswapV3Loader {
    pub fn new() -> Self {
        Self
    }
    
    /// 根据 factory 地址判断协议类型
    fn get_protocol_by_factory(factory: Address) -> PoolProtocol {
        if factory == *UNISWAP_V3_FACTORY {
            PoolProtocol::UniswapV3
        } else if factory == *SUSHISWAP_V3_FACTORY {
            PoolProtocol::SushiswapV3
        } else if factory == *PANCAKE_V3_FACTORY {
            PoolProtocol::PancakeV3
        } else {
            PoolProtocol::UniswapV3Like
        }
    }
}

#[async_trait]
impl PoolLoader for UniswapV3Loader {
    fn identify_pool_from_log(&self, log: &Log) -> Option<(PoolId, PoolClass)> {
        if log.topics().is_empty() {
            return None;
        }
        
        let topic0 = log.topics()[0];
        
        // 检查 UniswapV3 事件签名
        if topic0 == IUniswapV3Pool::Swap::SIGNATURE_HASH ||
           topic0 == IUniswapV3Pool::Mint::SIGNATURE_HASH ||
           topic0 == IUniswapV3Pool::Burn::SIGNATURE_HASH ||
           topic0 == IUniswapV3Pool::Initialize::SIGNATURE_HASH {
            Some((PoolId(log.address()), PoolClass::UniswapV3))
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
        let pool = IUniswapV3Pool::new(address, provider.clone());
        
        // 并行调用合约方法
        let (token0_result, token1_result, factory_result, fee_result, liquidity_result, slot0_result) = 
            tokio::join!(
                pool.token0(),
                pool.token1(),
                pool.factory(),
                pool.fee(),
                pool.liquidity(),
                pool.slot0()
            );
        
        let token0 = token0_result?._0;
        let token1 = token1_result?._0;
        let factory = factory_result?._0;
        let fee = fee_result?._0;
        let liquidity = liquidity_result?._0;
        let slot0 = slot0_result?;
        
        let protocol = Self::get_protocol_by_factory(factory);
        
        Ok(PoolData::UniswapV3(UniswapV3PoolData {
            address,
            token0,
            token1,
            factory,
            protocol,
            fee,
            liquidity,
            sqrt_price_x96: U256::from(slot0.sqrtPriceX96),
            tick: slot0.tick.try_into().unwrap_or(0),
        }))
    }
}
