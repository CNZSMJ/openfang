"""
验证昨日选股结果
对比S级股票的实际涨跌
"""

import requests
import pandas as pd
from datetime import datetime, timedelta

# 读取昨日选股结果
yesterday = (datetime.now() - timedelta(days=1)).strftime("%Y%m%d")
csv_file = f"选股结果_{yesterday}.csv"

try:
    df = pd.read_csv(csv_file)
    print(f"读取到昨日选股结果: {len(df)}只")
except:
    print(f"找不到 {csv_file}")
    exit()

# 取S级股票
s_stocks = df[df['rating'] == 'S级']
print(f"\n昨日S级: {len(s_stocks)}只")

# 获取今日涨跌
url = "https://push2.eastmoney.com/api/qt/clist/get"
codes = s_stocks['code'].tolist()
results = []

print("\n验证结果:")
for code in codes[:15]:
    params = {
        "pn": 1,
        "pz": 1,
        "po": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f3",
        "fs": f"b:{code}",
        "fields": "f2,f3,f14"
    }
    try:
        resp = requests.get(url, params=params, timeout=5)
        data = resp.json()
        if data.get('data') and data['data'].get('diff'):
            stock = data['data']['diff'][0]
            pct = stock.get('f3', 0)
            name = stock.get('f14', '')
            results.append({'code': code, 'name': name, 'pct': pct})
    except:
        pass

# 显示
for r in results:
    status = "✓涨停" if r['pct'] >= 9.9 else "✓大涨" if r['pct'] >= 5 else "✗不及预期" if r['pct'] >= 0 else "✗吃面"
    print(f"  {r['name']}({r['code']}): {r['pct']:+.2f}% {status}")

if results:
    avg_pct = sum(r['pct'] for r in results) / len(results)
    zt_count = sum(1 for r in results if r['pct'] >= 9.9)
    print(f"\n平均涨幅: {avg_pct:+.2f}% | 涨停: {zt_count}/{len(results)}")
