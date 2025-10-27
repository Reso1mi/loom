use alloy::primitives::{Address, U256};
use eyre::Result;
use std::collections::HashMap;
use tracing::debug;

use crate::state_update::{AccountState, StateUpdate, StateUpdateVec};
use crate::types::PoolData;

/// 简化的本地数据库
/// 存储合约的 storage 状态
#[derive(Debug, Clone, Default)]
pub struct LocalDB {
    /// address -> (slot -> value)
    storage: HashMap<Address, HashMap<U256, U256>>,
    /// address -> balance
    balances: HashMap<Address, U256>,
    /// address -> nonce
    nonces: HashMap<Address, u64>,
}

impl LocalDB {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// 读取 storage
    pub fn get_storage(&self, address: Address, slot: U256) -> Option<U256> {
        self.storage
            .get(&address)
            .and_then(|slots| slots.get(&slot))
            .copied()
    }
    
    /// 写入 storage
    pub fn set_storage(&mut self, address: Address, slot: U256, value: U256) {
        self.storage
            .entry(address)
            .or_insert_with(HashMap::new)
            .insert(slot, value);
    }
    
    /// 获取 balance
    pub fn get_balance(&self, address: Address) -> Option<U256> {
        self.balances.get(&address).copied()
    }
    
    /// 设置 balance
    pub fn set_balance(&mut self, address: Address, balance: U256) {
        self.balances.insert(address, balance);
    }
    
    /// 获取 nonce
    pub fn get_nonce(&self, address: Address) -> Option<u64> {
        self.nonces.get(&address).copied()
    }
    
    /// 设置 nonce
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.nonces.insert(address, nonce);
    }
    
    /// 应用状态更新
    pub fn apply_state_update(&mut self, update: &StateUpdate) {
        for (address, account_state) in update.iter() {
            // 更新 nonce
            if let Some(nonce) = account_state.nonce {
                self.set_nonce(*address, nonce);
            }
            
            // 更新 balance
            if let Some(balance) = account_state.balance {
                self.set_balance(*address, balance);
            }
            
            // 更新 storage
            for (slot, value) in account_state.storage.iter() {
                let slot_u256 = U256::from_be_bytes(slot.0);
                self.set_storage(*address, slot_u256, *value);
            }
        }
    }
    
    /// 应用多个状态更新
    pub fn apply_state_update_vec(&mut self, updates: &StateUpdateVec) {
        for update in updates.iter() {
            self.apply_state_update(update);
        }
    }
}

/// 市场状态
/// 管理所有池子和本地数据库
#[derive(Debug, Clone)]
pub struct MarketState {
    /// 本地数据库
    pub db: LocalDB,
    /// 已加载的池子 - address -> PoolData
    pub pools: HashMap<Address, PoolData>,
    /// 当前区块号
    pub current_block: u64,
}

impl MarketState {
    pub fn new() -> Self {
        Self {
            db: LocalDB::new(),
            pools: HashMap::new(),
            current_block: 0,
        }
    }
    
    /// 添加池子
    pub fn add_pool(&mut self, pool: PoolData) {
        let address = pool.address();
        debug!("Adding pool to market state: {:?}", address);
        self.pools.insert(address, pool);
    }
    
    /// 获取池子
    pub fn get_pool(&self, address: &Address) -> Option<&PoolData> {
        self.pools.get(address)
    }
    
    /// 检查是否是已知池子
    pub fn is_pool(&self, address: &Address) -> bool {
        self.pools.contains_key(address)
    }
    
    /// 应用状态更新
    pub fn apply_state_update(&mut self, update: &StateUpdate) {
        self.db.apply_state_update(update);
    }
    
    /// 应用多个状态更新
    pub fn apply_state_update_vec(&mut self, updates: &StateUpdateVec) {
        self.db.apply_state_update_vec(updates);
    }
    
    /// 更新当前区块
    pub fn update_block(&mut self, block_number: u64) {
        self.current_block = block_number;
    }
    
    /// 获取受影响的池子
    /// 从状态更新中识别哪些池子的状态发生了变化
    pub fn get_affected_pools(&self, update: &StateUpdate) -> Vec<Address> {
        let mut affected = Vec::new();
        
        for (address, _account_state) in update.iter() {
            if self.is_pool(address) {
                affected.push(*address);
            }
        }
        
        affected
    }
    
    /// 获取受影响的池子(从多个更新)
    pub fn get_affected_pools_vec(&self, updates: &StateUpdateVec) -> Vec<Address> {
        let mut affected = Vec::new();
        
        for update in updates.iter() {
            affected.extend(self.get_affected_pools(update));
        }
        
        // 去重
        affected.sort();
        affected.dedup();
        
        affected
    }
    
    /// 统计信息
    pub fn stats(&self) -> MarketStats {
        MarketStats {
            total_pools: self.pools.len(),
            current_block: self.current_block,
            storage_entries: self.db.storage.values().map(|s| s.len()).sum(),
        }
    }
}

impl Default for MarketState {
    fn default() -> Self {
        Self::new()
    }
}

/// 市场统计信息
#[derive(Debug, Clone)]
pub struct MarketStats {
    pub total_pools: usize,
    pub current_block: u64,
    pub storage_entries: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PoolClass, PoolData, UniswapV2PoolData, PoolProtocol};
    use alloy::primitives::B256;
    use std::collections::BTreeMap;
    
    #[test]
    fn test_local_db() {
        let mut db = LocalDB::new();
        let address = Address::repeat_byte(1);
        let slot = U256::from(0);
        let value = U256::from(100);
        
        db.set_storage(address, slot, value);
        assert_eq!(db.get_storage(address, slot), Some(value));
    }
    
    #[test]
    fn test_apply_state_update() {
        let mut db = LocalDB::new();
        let address = Address::repeat_byte(1);
        
        let mut update = StateUpdate::new();
        let mut storage = BTreeMap::new();
        storage.insert(B256::ZERO, U256::from(100));
        
        update.insert(address, AccountState {
            nonce: Some(1),
            balance: Some(U256::from(1000)),
            code: None,
            storage,
        });
        
        db.apply_state_update(&update);
        
        assert_eq!(db.get_nonce(address), Some(1));
        assert_eq!(db.get_balance(address), Some(U256::from(1000)));
        assert_eq!(db.get_storage(address, U256::ZERO), Some(U256::from(100)));
    }
    
    #[test]
    fn test_market_state() {
        let mut market = MarketState::new();
        
        let pool_address = Address::repeat_byte(1);
        let pool = PoolData::UniswapV2(UniswapV2PoolData {
            address: pool_address,
            token0: Address::repeat_byte(2),
            token1: Address::repeat_byte(3),
            factory: Address::repeat_byte(4),
            protocol: PoolProtocol::UniswapV2,
            fee: U256::from(30),
            reserve0: U256::from(1000),
            reserve1: U256::from(2000),
            reserves_slot: None,
        });
        
        market.add_pool(pool);
        assert!(market.is_pool(&pool_address));
        assert_eq!(market.pools.len(), 1);
    }
}
