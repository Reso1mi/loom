use alloy::rpc::types::Log;
use std::collections::HashMap;
use std::sync::Arc;

use crate::pool_loader_trait::PoolLoader;
use crate::types::{PoolClass, PoolId};

/// 池子加载器容器 - 简化版本
/// 
/// 管理多种类型的池子加载器，提供统一的接口
pub struct PoolLoaders {
    /// 存储不同类型的池子加载器
    /// Key: PoolClass, Value: 对应的 PoolLoader
    loaders: HashMap<PoolClass, Arc<dyn PoolLoader>>,
}

impl PoolLoaders {
    /// 创建新的 PoolLoaders
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
        }
    }
    
    /// 添加一个池子加载器
    pub fn add_loader(mut self, pool_class: PoolClass, loader: Arc<dyn PoolLoader>) -> Self {
        self.loaders.insert(pool_class, loader);
        self
    }
    
    /// 从日志中识别池子类型
    /// 
    /// 遍历所有加载器，找到第一个能识别该日志的加载器
    /// 返回 Some((pool_id, pool_class)) 如果识别成功
    pub fn identify_pool_from_log(&self, log: &Log) -> Option<(PoolId, PoolClass)> {
        for (_pool_class, loader) in self.loaders.iter() {
            if let Some(result) = loader.identify_pool_from_log(log) {
                return Some(result);
            }
        }
        None
    }
    
    /// 获取指定类型的加载器
    pub fn get_loader(&self, pool_class: &PoolClass) -> Option<Arc<dyn PoolLoader>> {
        self.loaders.get(pool_class).cloned()
    }
    
    /// 获取所有支持的池子类型
    pub fn supported_pool_classes(&self) -> Vec<PoolClass> {
        self.loaders.keys().copied().collect()
    }
}

impl Default for PoolLoaders {
    fn default() -> Self {
        Self::new()
    }
}
