#!/usr/bin/env python3
"""
SS v3.0 统一数据源管理器
综合 Tushare、AkShare、Baostock 三家数据源，互为备用
"""

# ==================== AkShare 代理补丁 ====================
# 必须在 import akshare 之前安装补丁
try:
    import akshare_proxy_patch
    akshare_proxy_patch.install_patch("101.201.173.125", "", 30)
    print("✓ AkShare 代理补丁已安装")
except ImportError:
    print("⚠ akshare-proxy-patch 未安装，使用默认方式")
except Exception as e:
    print(f"⚠ 代理补丁安装失败: {e}")

import pandas as pd
import numpy as np
from datetime import datetime, timedelta
from typing import Optional, Dict, List
import os
import json
import warnings
import logging

warnings.filterwarnings('ignore')

# 配置日志
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# ==================== 配置 ====================
CONFIG = {
    # Tushare 配置 (需要 Token)
    'tushare': {
        'token': os.getenv('TUSHARE_TOKEN', ''),
        'enabled': True,
        'priority': 1,
    },
    # AkShare 配置 (免费)
    'akshare': {
        'enabled': True,
        'priority': 2,
    },
    # Baostock 配置 (免费)
    'baostock': {
        'enabled': True,
        'priority': 3,
    },
    # 本地缓存配置
    'cache': {
        'enabled': True,
        'dir': os.path.join(os.path.dirname(__file__), 'data', 'cache'),
        'expire_days': 7,
    }
}

# 确保缓存目录存在
os.makedirs(CONFIG['cache']['dir'], exist_ok=True)
os.makedirs(os.path.join(CONFIG['cache']['dir'], 'daily'), exist_ok=True)
os.makedirs(os.path.join(CONFIG['cache']['dir'], 'financial'), exist_ok=True)
os.makedirs(os.path.join(CONFIG['cache']['dir'], 'money_flow'), exist_ok=True)
os.makedirs(os.path.join(CONFIG['cache']['dir'], 'stock_info'), exist_ok=True)


# ==================== 数据源基类 ====================
class BaseDataSource:
    """数据源基类"""
    
    name = "base"
    
    def __init__(self, config: Dict):
        self.config = config
        self.enabled = config.get('enabled', True)
        self.priority = config.get('priority', 999)
    
    def is_available(self) -> bool:
        """检查数据源是否可用"""
        raise NotImplementedError
    
    def get_daily(self, symbol: str, start_date: str, end_date: str) -> Optional[pd.DataFrame]:
        """获取日线数据"""
        raise NotImplementedError
    
    def get_stock_info(self, symbol: str) -> Optional[Dict]:
        """获取股票基本信息"""
        raise NotImplementedError
    
    def get_financial(self, symbol: str) -> Optional[pd.DataFrame]:
        """获取财务数据"""
        raise NotImplementedError
    
    def get_money_flow(self, symbol: str, date: str) -> Optional[pd.DataFrame]:
        """获取资金流向"""
        raise NotImplementedError


# ==================== Tushare 数据源 ====================
class TushareSource(BaseDataSource):
    """Tushare 数据源"""
    
    name = "tushare"
    
    def __init__(self, config: Dict):
        super().__init__(config)
        self.token = config.get('token', '')
        self.api = None
        
    def is_available(self) -> bool:
        if not self.enabled or not self.token:
            logger.info("Tushare: 未配置 Token")
            return False
        try:
            import tushare as ts
            ts.set_token(self.token)
            self.api = ts.pro_api()
            # 测试连接
            self.api.trade_cal(exchange='SSE', start_date='20240101', end_date='20240102')
            return True
        except Exception as e:
            logger.warning(f"Tushare 不可用: {e}")
            return False
    
    def _convert_code(self, symbol: str) -> str:
        """转换股票代码"""
        if symbol.startswith('6'):
            return f"{symbol}.SH"
        elif symbol.startswith(('0', '3')):
            return f"{symbol}.SZ"
        elif symbol.startswith('8') or symbol.startswith('4'):
            return f"{symbol}.BJ"
        return symbol
    
    def get_daily(self, symbol: str, start_date: str, end_date: str) -> Optional[pd.DataFrame]:
        if not self.api:
            return None
        try:
            ts_code = self._convert_code(symbol)
            df = self.api.daily(ts_code=ts_code, start_date=start_date, end_date=end_date)
            if df is not None and len(df) > 0:
                df = df.rename(columns={
                    'ts_code': 'symbol',
                    'trade_date': 'date',
                    'vol': 'volume'
                })
                df['date'] = pd.to_datetime(df['date']).dt.strftime('%Y%m%d')
                df = df.sort_values('date')
                return df
        except Exception as e:
            logger.warning(f"Tushare 日线失败: {symbol} - {e}")
        return None
    
    def get_stock_info(self, symbol: str) -> Optional[Dict]:
        if not self.api:
            return None
        try:
            ts_code = self._convert_code(symbol)
            df = self.api.stock_basic(ts_code=ts_code)
            if df is not None and len(df) > 0:
                return df.iloc[0].to_dict()
        except Exception as e:
            logger.warning(f"Tushare 基本信息失败: {symbol}")
        return None
    
    def get_financial(self, symbol: str) -> Optional[pd.DataFrame]:
        if not self.api:
            return None
        try:
            ts_code = self._convert_code(symbol)
            df = self.api.fina_indicator(ts_code=ts_code)
            if df is not None and len(df) > 0:
                return df
        except Exception as e:
            logger.warning(f"Tushare 财务数据失败: {symbol}")
        return None


# ==================== AkShare 数据源 ====================
class AkShareSource(BaseDataSource):
    """AkShare 数据源"""
    
    name = "akshare"
    
    def __init__(self, config: Dict):
        super().__init__(config)
        self.api = None
        
    def is_available(self) -> bool:
        if not self.enabled:
            return False
        try:
            import akshare as ak
            self.api = ak
            return True
        except Exception as e:
            logger.warning(f"AkShare 不可用: {e}")
            return False
    
    def _convert_symbol(self, symbol: str) -> tuple:
        """转换股票代码"""
        if symbol.startswith('6'):
            return symbol, 'sh'
        elif symbol.startswith(('0', '3')):
            return symbol, 'sz'
        return symbol, 'sh'
    
    def get_daily(self, symbol: str, start_date: str, end_date: str) -> Optional[pd.DataFrame]:
        if not self.api:
            return None
        try:
            sym, market = self._convert_symbol(symbol)
            # 转换日期格式
            start = datetime.strptime(start_date, "%Y%m%d").strftime("%Y%m%d")
            end = datetime.strptime(end_date, "%Y%m%d").strftime("%Y%m%d")
            
            df = self.api.stock_zh_a_hist(
                symbol=sym,
                period="daily",
                start_date=start,
                end_date=end,
                adjust=""
            )
            if df is not None and len(df) > 0:
                df = df.rename(columns={
                    '日期': 'date',
                    '开盘': 'open',
                    '收盘': 'close',
                    '最高': 'high',
                    '最低': 'low',
                    '成交量': 'volume',
                    '成交额': 'amount',
                    '振幅': 'amplitude',
                    '涨跌幅': 'pct_change',
                    '涨跌额': 'change',
                    '换手率': 'turnover'
                })
                df['date'] = pd.to_datetime(df['date']).dt.strftime('%Y%m%d')
                df['volume'] = df['volume'].astype(float)
                df = df.sort_values('date')
                return df
        except Exception as e:
            logger.warning(f"AkShare 日线失败: {symbol} - {e}")
        return None
    
    def get_stock_info(self, symbol: str) -> Optional[Dict]:
        if not self.api:
            return None
        try:
            sym, _ = self._convert_symbol(symbol)
            df = self.api.stock_individual_info_em(symbol=sym)
            if df is not None and len(df) > 0:
                return dict(zip(df['item'].tolist(), df['value'].tolist()))
        except Exception as e:
            logger.warning(f"AkShare 基本信息失败: {symbol}")
        return None
    
    def get_money_flow(self, symbol: str, date: str = None) -> Optional[pd.DataFrame]:
        if not self.api:
            return None
        try:
            sym, market = self._convert_symbol(symbol)
            df = self.api.stock_individual_fund_flow(stock=sym, market=market)
            if df is not None and len(df) > 0:
                return df
        except Exception as e:
            logger.warning(f"AkShare 资金流向失败: {symbol}")
        return None


# ==================== Baostock 数据源 ====================
class BaostockSource(BaseDataSource):
    """Baostock 数据源"""
    
    name = "baostock"
    
    def __init__(self, config: Dict):
        super().__init__(config)
        self.api = None
        self._logged_in = False
        
    def is_available(self) -> bool:
        if not self.enabled:
            return False
        try:
            import baostock as bs
            self.api = bs
            lg = bs.login()
            if lg.error_code == '0':
                self._logged_in = True
                return True
            return False
        except Exception as e:
            logger.warning(f"Baostock 不可用: {e}")
            return False
    
    def _convert_code(self, symbol: str) -> str:
        """转换股票代码"""
        if symbol.startswith('6'):
            return f"sh.{symbol}"
        elif symbol.startswith(('0', '3')):
            return f"sz.{symbol}"
        return f"sh.{symbol}"
    
    def get_daily(self, symbol: str, start_date: str, end_date: str) -> Optional[pd.DataFrame]:
        if not self.api:
            return None
        try:
            bs_code = self._convert_code(symbol)
            
            # 转换日期格式
            start = datetime.strptime(start_date, "%Y%m%d").strftime("%Y-%m-%d")
            end = datetime.strptime(end_date, "%Y%m%d").strftime("%Y-%m-%d")
            
            fields = "date,code,open,high,low,close,volume,amount"
            rs = self.api.query_history_k_data_plus(bs_code, fields, 
                                                     start_date=start, end_date=end)
            data_list = []
            while (rs.error_code == '0') and rs.next():
                data_list.append(rs.get_row_data())
            
            if data_list:
                df = pd.DataFrame(data_list, columns=rs.fields)
                df = df.rename(columns={'code': 'symbol'})
                df['symbol'] = df['symbol'].str.replace('sh.', '').str.replace('sz.', '')
                df['date'] = pd.to_datetime(df['date']).dt.strftime('%Y%m%d')
                # 转换数值类型
                for col in ['open', 'high', 'low', 'close', 'volume', 'amount']:
                    df[col] = pd.to_numeric(df[col], errors='coerce')
                df = df.sort_values('date')
                return df
        except Exception as e:
            logger.warning(f"Baostock 日线失败: {symbol} - {e}")
        return None
    
    def get_stock_info(self, symbol: str) -> Optional[Dict]:
        if not self.api:
            return None
        try:
            bs_code = self._convert_code(symbol)
            rs = self.api.query_stock_basic(code=bs_code)
            data_list = []
            while (rs.error_code == '0') and rs.next():
                data_list.append(rs.get_row_data())
            
            if data_list:
                return dict(zip(rs.fields, data_list[0]))
        except Exception as e:
            logger.warning(f"Baostock 基本信息失败: {symbol}")
        return None
    
    def get_financial(self, symbol: str) -> Optional[pd.DataFrame]:
        # Baostock 不直接提供财务数据
        return None
    
    def __del__(self):
        """登出"""
        if self.api and self._logged_in:
            try:
                self.api.logout()
            except:
                pass


# ==================== 本地缓存 ====================
class CacheManager:
    """本地缓存管理"""
    
    def __init__(self, config: Dict):
        self.cache_dir = config['dir']
        self.expire_days = config['expire_days']
    
    def _get_cache_path(self, type: str, symbol: str) -> str:
        return os.path.join(self.cache_dir, type, f"{symbol}.csv")
    
    def get(self, type: str, symbol: str, start_date: str = None) -> Optional[pd.DataFrame]:
        """读取缓存"""
        cache_file = self._get_cache_path(type, symbol)
        if not os.path.exists(cache_file):
            return None
        
        # 检查过期
        mtime = datetime.fromtimestamp(os.path.getmtime(cache_file))
        if (datetime.now() - mtime).days > self.expire_days:
            return None
        
        try:
            df = pd.read_csv(cache_file)
            if start_date and 'date' in df.columns:
                df = df[df['date'] >= start_date]
            return df
        except:
            return None
    
    def save(self, type: str, symbol: str, df: pd.DataFrame):
        """保存缓存"""
        if df is None or len(df) == 0:
            return
        cache_file = self._get_cache_path(type, symbol)
        os.makedirs(os.path.dirname(cache_file), exist_ok=True)
        df.to_csv(cache_file, index=False)


# ==================== 统一数据管理器 ====================
class DataManager:
    """
    统一数据管理器
    
    使用策略:
    1. 优先使用 Tushare (速度快，字段全)
    2. 备用 AkShare (数据丰富，实时性好) - 现在有代理补丁
    3. 备用 Baostock (稳定，复权准确)
    4. 最后使用缓存
    """
    
    def __init__(self, config: Dict = None):
        self.config = config or CONFIG
        self.sources = []
        self.cache = CacheManager(self.config['cache'])
        
        # 初始化数据源
        self._init_sources()
        
    def _init_sources(self):
        """初始化数据源"""
        # Tushare
        ts_source = TushareSource(self.config['tushare'])
        if ts_source.is_available():
            self.sources.append(ts_source)
            logger.info("✓ Tushare 已加载 (主数据源)")
        
        # AkShare
        ak_source = AkShareSource(self.config['akshare'])
        if ak_source.is_available():
            self.sources.append(ak_source)
            logger.info("✓ AkShare 已加载 (备用) - 带代理补丁")
        
        # Baostock
        bs_source = BaostockSource(self.config['baostock'])
        if bs_source.is_available():
            self.sources.append(bs_source)
            logger.info("✓ Baostock 已加载 (备用)")
        
        # 按优先级排序
        self.sources.sort(key=lambda x: x.priority)
        
        if not self.sources:
            logger.warning("⚠ 没有可用的数据源!")
    
    def _try_sources(self, method: str, *args, **kwargs) -> any:
        """依次尝试各数据源"""
        errors = []
        
        for source in self.sources:
            try:
                method_func = getattr(source, method, None)
                if method_func:
                    result = method_func(*args, **kwargs)
                    if result is not None and len(result) > 0:
                        logger.info(f"✓ {source.name} {method} 成功 ({len(result) if hasattr(result, '__len__') else 1} 条)")
                        return result
            except Exception as e:
                error_msg = f"{source.name}: {str(e)[:50]}"
                errors.append(error_msg)
                logger.debug(f"✗ {source.name} {method} 失败: {e}")
        
        # 所有源都失败
        if errors:
            logger.warning(f"{method} 所有数据源失败")
        return None
    
    # ==================== 对外接口 ====================
    
    def get_daily(self, symbol: str, start_date: str, end_date: str) -> Optional[pd.DataFrame]:
        """
        获取日线数据
        
        优先级: Tushare → AkShare → Baostock → 缓存
        """
        # 先尝试缓存
        if self.config['cache']['enabled']:
            cached = self.cache.get('daily', symbol, start_date)
            if cached is not None:
                # 过滤日期范围
                cached = cached[(cached['date'] >= start_date) & (cached['date'] <= end_date)]
                if len(cached) > 0:
                    logger.info(f"✓ 缓存命中: {symbol} ({len(cached)}条)")
                    return cached
        
        # 尝试各数据源
        df = self._try_sources('get_daily', symbol, start_date, end_date)
        
        # 保存到缓存
        if df is not None and self.config['cache']['enabled']:
            self.cache.save('daily', symbol, df)
        
        return df
    
    def get_stock_info(self, symbol: str) -> Optional[Dict]:
        """获取股票基本信息"""
        # 先尝试缓存
        if self.config['cache']['enabled']:
            cached = self.cache.get('stock_info', symbol)
            if cached is not None:
                logger.info(f"✓ 股票信息缓存命中: {symbol}")
                return cached.to_dict('records')[0] if len(cached) > 0 else None
        
        # 尝试数据源
        result = self._try_sources('get_stock_info', symbol)
        
        # 保存到缓存
        if result is not None and self.config['cache']['enabled']:
            df = pd.DataFrame([result])
            self.cache.save('stock_info', symbol, df)
        
        return result
    
    def get_financial(self, symbol: str) -> Optional[pd.DataFrame]:
        """获取财务数据"""
        # 先尝试缓存
        if self.config['cache']['enabled']:
            cached = self.cache.get('financial', symbol)
            if cached is not None:
                logger.info(f"✓ 财务缓存命中: {symbol}")
                return cached
        
        # 尝试数据源
        result = self._try_sources('get_financial', symbol)
        
        # 保存到缓存
        if result is not None and self.config['cache']['enabled']:
            self.cache.save('financial', symbol, result)
        
        return result
    
    def get_money_flow(self, symbol: str, date: str = None) -> Optional[pd.DataFrame]:
        """获取资金流向"""
        # 先尝试缓存
        cache_key = f"money_flow_{symbol}"
        if self.config['cache']['enabled']:
            cached = self.cache.get('money_flow', f"{symbol}_{date or 20}d")
            if cached is not None:
                logger.info(f"✓ 资金流向缓存命中: {symbol}")
                return cached
        
        # AkShare 特有
        for source in self.sources:
            if source.name == 'akshare':
                try:
                    result = source.get_money_flow(symbol, date)
                    if result is not None:
                        # 保存到缓存
                        if self.config['cache']['enabled']:
                            self.cache.save('money_flow', f"{symbol}_{date or 20}d", result)
                        return result
                except:
                    pass
        return None
    
    def get_recent_daily(self, symbol: str, days: int = 30) -> Optional[pd.DataFrame]:
        """获取最近N天日线"""
        end = datetime.now()
        start = end - timedelta(days=days * 2)  # 多取一些，防止非交易日
        return self.get_daily(symbol, start.strftime('%Y%m%d'), end.strftime('%Y%m%d'))
    
    def get_all_stocks(self) -> List[str]:
        """获取所有股票列表"""
        # 优先使用 AkShare
        for source in self.sources:
            if source.name == 'akshare':
                try:
                    df = source.api.stock_zh_a_spot_em()
                    if df is not None:
                        return df['代码'].tolist()[:100]
                except:
                    pass
        
        # 备用: 返回常用股票
        return [
            '600519', '000858', '300750', '002594', '600036',
            '000333', '601318', '000001', '600900', '688008',
            '000651', '600276', '600887', '000725', '002415'
        ]
    
    def health_check(self) -> Dict:
        """健康检查"""
        result = {}
        for source in self.sources:
            result[source.name] = {
                'available': True,
                'priority': source.priority
            }
        return result


# ==================== 便捷函数 ====================
def get_dm() -> DataManager:
    """获取数据管理器单例"""
    global _dm_instance
    if '_dm_instance' not in globals():
        _dm_instance = DataManager()
    return _dm_instance


# 测试
if __name__ == "__main__":
    print("=" * 60)
    print("数据源健康检查")
    print("=" * 60)
    
    dm = DataManager()
    health = dm.health_check()
    
    print("\n可用数据源:")
    for name, info in health.items():
        status = "✓" if info['available'] else "✗"
        print(f"  {status} {name} (优先级: {info['priority']})")
    
    print("\n" + "-" * 60)
    print("测试获取日线数据 (600519 茅台)")
    print("-" * 60)
    
    df = dm.get_daily('600519', '20240101', '20240308')
    if df is not None:
        print(f"✓ 成功获取 {len(df)} 条数据")
        print(df.tail(3)[['date', 'close', 'volume']])
    else:
        print("✗ 获取失败")
    
    print("\n" + "-" * 60)
    print("测试获取资金流向 (000001 平安银行)")
    print("-" * 60)
    
    flow = dm.get_money_flow('000001')
    if flow is not None:
        print(f"✓ 成功获取 {len(flow)} 条数据")
        print(flow.head(2))
    else:
        print("✗ 获取失败 (AkShare 需要网络通畅)")
