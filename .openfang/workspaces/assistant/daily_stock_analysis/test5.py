import akshare as ak

# 看涨停池返回什么数据
df = ak.stock_zt_pool_em(date="20260309")
print("涨停池列:", df.columns.tolist())
print("涨停池形状:", df.shape)
print("\n前3行:")
print(df[['名称', '代码', '涨跌幅', '首次封板时间', '炸板次数', '所属行业']].head(3))

# 看看有没有跌停的（跌停的不在涨停池里）
# 试试用其他方式获取涨跌停统计
try:
    df2 = ak.stock_zt_pool_strong_em(date="20260309")
    print("\n涨停池(强):", df2.columns.tolist())
except Exception as e:
    print(f"error: {e}")
