import akshare as ak
import pandas as pd

# 搜索全市场统计接口
print("搜索全市场统计...")
for name in dir(ak):
    if 'market' in name.lower() or 'overview' in name.lower() or '统计' in name or '总' in name:
        if 'em' in name:
            print(name)

# 试试一些可能的接口
try:
    df = ak.stock_zh_index_daily_em()
    print("\n指数日线:", df.columns.tolist())
except Exception as e:
    print(f"error: {e}")

# 东方财富行情中心
try:
    df = ak.stock_zh_a_allem_code()
    print("\n所有股票:", df.columns.tolist())
except Exception as e:
    print(f"error: {e}")
