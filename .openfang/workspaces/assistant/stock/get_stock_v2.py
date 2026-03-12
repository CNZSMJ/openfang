import akshare as ak
import warnings
warnings.filterwarnings("ignore")

# 直接获取最新数据
print("=== 获取最新数据 ===")

# 涨跌停统计
df_zt = ak.stock_zt_pool_em(date="20260309")
df_zt_pool = ak.stock_zt_pool_strong_em(date="20260309")

print(f"涨停: {len(df_zt)}")
print(f"炸板(含涨停): {len(df_zt_pool)}")
print(f"封板率: {len(df_zt)/len(df_zt_pool)*100:.1f}%")

# 跌停
df = ak.stock_zh_a_spot_em()
df_dt = df[df["涨跌幅"] <= -9.9]
print(f"跌停: {len(df_dt)}")
print(df_dt[["代码", "名称", "涨跌幅"]].to_string())
