"""
验证：用东财API获取股票行业(f100)，能否和资金流向API匹配
"""
import requests
import json
import re
import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
import akshare as ak
from eastmoney_money_flow import get_board_money_flow

date = '20260310'

# 1. 涨停板 - 用akshare获取股票代码和所属行业
df_zt = ak.stock_zt_pool_em(date=date)
zt_stocks = df_zt[['代码', '名称', '所属行业']].head(10)

# 2. 用东财API获取这些股票的行业(f100)
headers = {"User-Agent": "Mozilla/5.0"}

print("=== 对比：akshare所属行业 vs 东财f100行业 ===\n")

for _, row in zt_stocks.iterrows():
    code = row['代码']
    akshare_board = row['所属行业']
    
    # 东财接口
    url = "https://push2.eastmoney.com/api/qt/stock/get"
    params = {
        "fltt": 2,
        "fields": "f100,f14",
        "secid": f"0.{code}"  # 深圳
    }
    
    resp = requests.get(url, params=params, headers=headers, timeout=5)
    try:
        data = resp.json()
        em_board = data.get('data', {}).get('f100', 'N/A')
        print(f"{code} {row['名称']}")
        print(f"  akshare: {akshare_board}")
        print(f"  东财f100: {em_board}")
        print()
    except:
        print(f"{code}: 接口错误")
