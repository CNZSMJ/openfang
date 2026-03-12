"""
最终验证：akshare所属行业 vs 东财f127行业 vs 资金流向板块
"""
import requests
import json
import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
import akshare as ak
from eastmoney_money_flow import get_board_money_flow

date = '20260310'

# 1. 获取涨停板数据
df_zt = ak.stock_zt_pool_em(date=date)

# 2. 获取资金流向板块
mf = get_board_money_flow(30)
mf_boards = set([item['板块名'] for item in mf])

# 3. 用东财API获取每只涨停板的f127行业
headers = {"User-Agent": "Mozilla/5.0"}

def get_f127(code):
    """获取东财f127行业"""
    prefix = '0' if code.startswith('00') or code.startswith('30') else '1'
    url = "https://push2.eastmoney.com/api/qt/stock/get"
    params = {
        "fltt": 2,
        "fields": "f127",
        "secid": f"{prefix}.{code}"
    }
    try:
        resp = requests.get(url, params=params, headers=headers, timeout=3)
        data = resp.json()
        return data.get('data', {}).get('f127', '')
    except:
        return ''

# 获取涨停板的f127行业
zt_with_f127 = []
for _, row in df_zt.iterrows():
    code = row['代码']
    f127 = get_f127(code)
    zt_with_f127.append({
        'code': code,
        'name': row['名称'],
        'akshare_board': row['所属行业'],
        'f127_board': f127
    })

# 统计f127行业
from collections import Counter
f127_counts = Counter([x['f127_board'] for x in zt_with_f127 if x['f127_board']])
print("=== 涨停板行业分布(东财f127) ===")
print(f127_counts.most_common(10))

print("\n=== 资金流向板块 ===")
print(sorted(mf_boards))

# 交集
f127_set = set(f127_counts.keys())
intersection = f127_set & mf_boards
print(f"\n=== f127与资金流向交集 ===")
print(sorted(intersection))
