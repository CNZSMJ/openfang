import akshare as ak
import pandas as pd

# 炸板股池
try:
    df = ak.stock_zt_pool_zbgc_em(date="20260309")
    print("炸板股池列:", df.columns.tolist())
    print(f"数量: {len(df)}")
    print(df[['名称', '代码', '涨跌幅', '最高价', '涨停价']].head(5))
except Exception as e:
    print(f"炸板股池error: {e}")

# 昨日涨停
try:
    df = ak.stock_zt_pool_previous_em(date="20260309")
    print("\n昨日涨停列:", df.columns.tolist())
    print(f"数量: {len(df)}")
except Exception as e:
    print(f"昨日涨停error: {e}")

# 龙池股
try:
    df = ak.stock_zt_pool_dtgc_em(date="20260309")
    print("\n龙头股池列:", df.columns.tolist())
    print(f"数量: {len(df)}")
except Exception as e:
    print(f"龙头股池error: {e}")
