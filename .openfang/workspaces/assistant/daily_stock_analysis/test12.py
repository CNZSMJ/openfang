import akshare as ak
import pandas as pd

# 看看炸板股池的数据结构
df = ak.stock_zt_pool_zbgc_em(date="20260309")
print("炸板股池列:", df.columns.tolist())
print(f"数量: {len(df)}")
print("\n前10行:")
print(df[['名称', '代码', '涨跌幅', '最新价', '涨停价']].head(10))

# 看看涨跌幅分布
print("\n涨跌幅分布:")
print(df['涨跌幅'].describe())

# 看看接近涨停的（冲高未涨停）
# 炸板股里有一些是收盘涨7-9%的，属于冲高未涨停
chonggao = df[(df['涨跌幅'] > 7) & (df['涨跌幅'] < 9.5)]
print(f"\n冲高未涨停(7%~9.5%): {len(chonggao)}只")
print(chonggao[['名称', '代码', '涨跌幅', '最新价', '涨停价', '所属行业']])
