#!/usr/bin/env python3
"""
SS v4.0 数据源模块 - 支持历史时点数据
"""

import os
import json
import pandas as pd
import numpy as np
from datetime import datetime, timedelta
from pathlib import Path

try:
    import akshare as ak
    AKSHARE_AVAILABLE = True
except ImportError:
    AKSHARE_AVAILABLE = False
    print("警告: 未安装akshare")

CACHE_DIR = Path(__file__).parent / "cache"
CACHE_DIR.mkdir(exist_ok=True)


def parse_number(x):
    """
    智能解析数字，支持:
    - 普通数字: 123, 12.34
    - 百分比: 15.6%, 16.5
    - 单位: 1.23亿, 4.56万, 7890
    返回: float 或 None
    """
    if x is None or pd.isna(x):
        return None
    
    if isinstance(x, (int, float)):
        return float(x)
    
    if isinstance(x, str):
        x = x.strip()
        
        # 处理百分比
        if '%' in x:
            try:
                return float(x.replace('%', ''))
            except:
                return None
        
        # 处理单位
        try:
            if '亿' in x:
                return float(x.replace('亿', '').strip()) * 1e8
            elif '万' in x:
                return float(x.replace('万', '').strip()) * 1e4
            elif '千' in x:
                return float(x.replace('千', '').strip()) * 1e3
            else:
                return float(x)
        except:
            return None
    
    return None


def parse_pct(x):
    """解析百分比数值"""
    if x is None or pd.isna(x):
        return None
    
    if isinstance(x, (int, float)):
        return float(x)
    
    if isinstance(x, str):
        x = x.strip()
        
        # 如果已经是小数形式 (如 0.15)，转为百分比
        if '.' in x and '%' not in x:
            try:
                v = float(x)
                if v < 1:  # 小于1可能是小数形式
                    return v * 100
                return v
            except:
                return None
        
        # 带百分号
        if '%' in x:
            try:
                return float(x.replace('%', ''))
            except:
                return None
        
        # 带单位
        try:
            if '亿' in x:
                return float(x.replace('亿', '').strip()) * 1e8
            elif '万' in x:
                return float(x.replace('万', '').strip()) * 1e4
            else:
                return float(x)
        except:
            return None
    
    return None


class DataManagerV4:
    """数据管理器 v4.0 - 支持历史时点数据查询"""
    
    def __init__(self):
        self.cache_dir = CACHE_DIR
        self._financial_cache = {}
        self._load_financial_cache()
        self._daily_cache = {}  # 内存缓存
    
    def _load_financial_cache(self):
        cache_file = self.cache_dir / "financial_history.json"
        if cache_file.exists():
            with open(cache_file, 'r') as f:
                self._financial_cache = json.load(f)
        else:
            self._financial_cache = {}
    
    def _save_financial_cache(self):
        cache_file = self.cache_dir / "financial_history.json"
        with open(cache_file, 'w') as f:
            json.dump(self._financial_cache, f, ensure_ascii=False, indent=2)
    
    def get_daily(self, symbol, start_date=None, end_date=None, adjust="qfq"):
        """获取日线数据"""
        return self._get_daily_fast(symbol, start_date, end_date, adjust)
    
    def _get_daily_fast(self, symbol, start_date, end_date, adjust):
        """快速获取日线数据"""
        cache_key = f"{symbol}_{adjust}"
        
        # 尝试内存缓存
        if cache_key in self._daily_cache:
            df = self._daily_cache[cache_key]
        else:
            # 从网络获取全部历史数据
            try:
                df = ak.stock_zh_a_hist(symbol=symbol, adjust=adjust)
                if df is None or len(df) == 0:
                    return None
                
                # 统一列名
                df.columns = ['日期', '股票代码', '开盘', '收盘', '最高', '最低', '成交量', '成交额', '振幅', '涨跌幅', '涨跌额', '换手率']
                df['日期'] = pd.to_datetime(df['日期'])
                df['成交量'] = pd.to_numeric(df['成交量'], errors='coerce')
                df['成交额'] = pd.to_numeric(df['成交额'], errors='coerce')
                df['涨跌幅'] = pd.to_numeric(df['涨跌幅'], errors='coerce')
                
                # 内存缓存
                self._daily_cache[cache_key] = df
                print(f"  ✓ {symbol} 加载 {len(df)} 条日线数据")
                
            except Exception as e:
                print(f"  ✗ {symbol} 日线数据获取失败: {e}")
                return None
        
        # 日期过滤
        if start_date:
            start_dt = pd.to_datetime(start_date[:4] + '-' + start_date[4:6] + '-' + start_date[6:])
            df = df[df['日期'] >= start_dt]
        
        if end_date:
            end_dt = pd.to_datetime(end_date[:4] + '-' + end_date[4:6] + '-' + end_date[6:])
            df = df[df['日期'] <= end_dt]
        
        return df
    
    def get_financial_at_date(self, symbol, target_date):
        """获取指定日期的财务数据（历史时点）"""
        target = datetime.strptime(str(target_date), "%Y%m%d")
        year = target.year
        month = target.month
        
        # 确定使用哪期财报
        if month <= 4:
            report_period = f"{year-1}Q4"
        elif month <= 8:
            report_period = f"{year-1}Q4"
        elif month <= 10:
            report_period = f"{year}Q1"
        else:
            report_period = f"{year}Q2"
        
        cache_key = f"{symbol}_{report_period}"
        if cache_key in self._financial_cache:
            return self._financial_cache[cache_key]
        
        data = self._fetch_financial(symbol, report_period)
        
        if data:
            self._financial_cache[cache_key] = data
            self._save_financial_cache()
        
        return data
    
    def _fetch_financial(self, symbol, report_period):
        """从akshare获取财务数据"""
        try:
            df = ak.stock_financial_abstract_ths(symbol=symbol)
            
            if df is None or len(df) == 0:
                return None
            
            # 解析报告期
            df['report_dt'] = pd.to_datetime(df['报告期'], errors='coerce')
            df = df.dropna(subset=['report_dt'])
            
            if len(df) == 0:
                return None
            
            target_dt = self._parse_report_period(report_period)
            if target_dt is None:
                return None
            
            # 找最近的一期
            df = df[df['report_dt'] <= target_dt].sort_values('report_dt', ascending=False)
            
            if len(df) == 0:
                return None
            
            row = df.iloc[0]
            
            return {
                'symbol': symbol,
                'report_period': report_period,
                'report_date': str(row['报告期']),
                'roe': parse_pct(row.get('净资产收益率', None)),
                'revenue_growth': parse_pct(row.get('营业总收入同比增长率', None)),
                'profit_growth': parse_pct(row.get('净利润同比增长率', None)),
                'gross_margin': parse_pct(row.get('销售毛利率', None)),
                'net_margin': parse_pct(row.get('销售净利率', None)),
                'revenue': parse_number(row.get('营业总收入', None)),
                'net_profit': parse_number(row.get('净利润', None)),
                # 新增: 估值相关字段 (如果有)
                'pe_ttm': parse_number(row.get('市盈率TTM', None)),
                'dividend_yield': parse_pct(row.get('股息率', None)),
                'pb': parse_number(row.get('市净率', None)),
            }
            
        except Exception as e:
            print(f"  ✗ {symbol} 财务数据获取失败: {e}")
            return None
    
    def _parse_report_period(self, report_period):
        try:
            year = int(report_period[:4])
            quarter = int(report_period[5])
            month = quarter * 3
            return datetime(year, month, 1)
        except:
            return None
    
    def get_market_data(self, symbol, date):
        """获取指定日期的市场动量数据"""
        end_date = str(date)
        start_date = self._days_before(date, 90)
        
        df = self.get_daily(symbol, start_date, end_date)
        
        if df is None or len(df) < 30:
            return None
        
        df = df.sort_values('日期')
        
        current_price = df.iloc[-1]['收盘']
        price_20d_ago = df.iloc[-21]['收盘'] if len(df) > 20 else df.iloc[0]['收盘']
        price_60d_ago = df.iloc[-61]['收盘'] if len(df) > 60 else df.iloc[0]['收盘']
        
        return {
            'close': current_price,
            'change_20d': (current_price - price_20d_ago) / price_20d_ago * 100,
            'change_60d': (current_price - price_60d_ago) / price_60d_ago * 100,
            'volume_20d': df.tail(20)['成交量'].mean(),
            'amount_20d': df.tail(20)['成交额'].mean(),
            'volatility_20d': df.tail(20)['涨跌幅'].std() * np.sqrt(252) if len(df) > 20 else 0,
        }
    
    def _days_before(self, date_str, days):
        date = datetime.strptime(str(date_str), "%Y%m%d")
        before = date - timedelta(days=days)
        return before.strftime("%Y%m%d")


# 测试
if __name__ == "__main__":
    dm = DataManagerV4()
    
    print("=" * 50)
    print("测试1: 数字解析")
    print(f"  '1.03万' -> {parse_number('1.03万')}")
    print(f"  '15.6%' -> {parse_pct('15.6%')}")
    print(f"  '0.156' -> {parse_pct('0.156')}")
    
    print("\n" + "=" * 50)
    print("测试2: 日线数据")
    df = dm.get_daily("600519", "20231201", "20231231")
    if df is not None:
        print(f"  茅台12月日线: {len(df)} 条")
    
    print("\n" + "=" * 50)
    print("测试3: 财务数据")
    fin = dm.get_financial_at_date("600519", "20250926")
    if fin:
        print(f"  报告期: {fin.get('report_date')}")
        print(f"  ROE: {fin.get('roe')}")
        print(f"  PE: {fin.get('pe_ttm')}")
    
    print("\n✓ 测试完成!")
