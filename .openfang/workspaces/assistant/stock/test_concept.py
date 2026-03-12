"""
测试概念板块能否匹配
"""
import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
import akshare as ak
import requests
import json
import re

date = '20260310'

# 1. 涨停板板块
df_zt = ak.stock_zt_pool_em(date=date)
zt_boards = set(df_zt['所属行业'].unique())

# 2. 测试概念板块
headers = {"User-Agent": "Mozilla/5.0"}
url = "https://push2.eastmoney.com/api/qt/clist/get"
params = {
    "pn": 1,
    "pz": 50,
    "po": 1,
    "np": 1,
    "fltt": 2,
    "invt": 2,
    "fid": "f62",
    "fs": "m:90+t:3",  # 概念板块
    "fields": "f12,f14,f2,f3,f62"
}

resp = requests.get(url, params=params, headers=headers, timeout=10)
text = resp.text
match = re.search(r'\{.*\}', text)
data = json.loads(match.group(0))
concept_boards = set([item['f14'] for item in data['data']['diff']])

print("=== 涨停板板块(akshare) ===")
print(sorted(zt_boards))

print("\n=== 概念板块(东财) ===")
print(sorted(concept_boards))

# 3. 交集
intersection = zt_boards & concept_boards
print(f"\n=== 涨停板与概念板块交集 ===")
print(sorted(intersection))
