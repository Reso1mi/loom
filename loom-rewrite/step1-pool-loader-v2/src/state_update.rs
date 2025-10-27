use alloy::primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 账户状态变化
/// 对应 Geth 的 debug_traceBlock 返回的状态变化
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountState {
    /// nonce 变化
    pub nonce: Option<u64>,
    /// balance 变化
    pub balance: Option<U256>,
    /// code 变化 (新部署的合约)
    pub code: Option<Bytes>,
    /// storage 变化 - slot -> value
    /// 这是最关键的部分,包含了所有池子的状态变化
    pub storage: BTreeMap<B256, U256>,
}

/// 单个区块的状态更新
/// Address -> AccountState
pub type StateUpdate = BTreeMap<Address, AccountState>;

/// 多个交易的状态更新向量
/// 一个区块内可能有多个交易,每个交易都有自己的状态变化
pub type StateUpdateVec = Vec<StateUpdate>;

/// 区块状态更新消息
#[derive(Debug, Clone)]
pub struct BlockStateUpdate {
    /// 区块号
    pub block_number: u64,
    /// 区块哈希
    pub block_hash: B256,
    /// 状态更新
    pub state_update: StateUpdateVec,
}

impl BlockStateUpdate {
    pub fn new(block_number: u64, block_hash: B256, state_update: StateUpdateVec) -> Self {
        Self {
            block_number,
            block_hash,
            state_update,
        }
    }
}

/// 获取状态更新中涉及的所有地址
pub fn get_touched_addresses(state_update: &StateUpdate) -> Vec<Address> {
    state_update.keys().copied().collect()
}

/// 合并多个状态更新
pub fn merge_state_updates(updates: Vec<StateUpdate>) -> StateUpdate {
    let mut merged = StateUpdate::new();
    
    for update in updates {
        for (address, account_state) in update {
            let entry = merged.entry(address).or_insert_with(AccountState::default);
            
            // 合并 nonce
            if let Some(nonce) = account_state.nonce {
                entry.nonce = Some(nonce);
            }
            
            // 合并 balance
            if let Some(balance) = account_state.balance {
                entry.balance = Some(balance);
            }
            
            // 合并 code
            if let Some(code) = account_state.code {
                entry.code = Some(code);
            }
            
            // 合并 storage
            for (slot, value) in account_state.storage {
                entry.storage.insert(slot, value);
            }
        }
    }
    
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_merge_state_updates() {
        let addr1 = Address::repeat_byte(1);
        let addr2 = Address::repeat_byte(2);
        
        let mut update1 = StateUpdate::new();
        update1.insert(addr1, AccountState {
            nonce: Some(1),
            balance: Some(U256::from(100)),
            code: None,
            storage: BTreeMap::new(),
        });
        
        let mut update2 = StateUpdate::new();
        update2.insert(addr1, AccountState {
            nonce: Some(2),
            balance: None,
            code: None,
            storage: BTreeMap::new(),
        });
        update2.insert(addr2, AccountState {
            nonce: Some(1),
            balance: Some(U256::from(200)),
            code: None,
            storage: BTreeMap::new(),
        });
        
        let merged = merge_state_updates(vec![update1, update2]);
        
        assert_eq!(merged.len(), 2);
        assert_eq!(merged.get(&addr1).unwrap().nonce, Some(2));
        assert_eq!(merged.get(&addr1).unwrap().balance, Some(U256::from(100)));
        assert_eq!(merged.get(&addr2).unwrap().nonce, Some(1));
    }
}
