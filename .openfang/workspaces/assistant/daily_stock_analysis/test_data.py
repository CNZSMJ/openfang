import akshare as ak
import pandas as pd

# 测试各种数据接口
DATE = "20260309"

# 1. 涨跌比
try:
    df = ak.stock_zh_a_em_analytical(symbol="上涨下跌")
    print("涨跌比:", df)
except Exception as e:
    print(f"涨跌比error: {e}")

# 2. 跌停池
try:
    df = ak.stock_zt_pool_em(date=DATE, pool="跌停")
    print("\n跌停池:", df.columns.tolist())
    print(df.head(3))
except Exception as e:
    print(f"跌停池error: {e}")

# 3. 冲高未涨停（炸板股）
try:
    df = ak.stock_zt_pool_em(date=DATE, pool="炸板")
    print("\n炸板池:", df.columns.tolist())
except Exception as e:
    print(f"炸板池error: {e}")
