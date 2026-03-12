import akshare as ak
import warnings
warnings.filterwarnings("ignore")

# 获取市场总成交额
print("=== 获取成交额 ===")

# 上证 - 取最后一行
sh = ak.stock_zh_index_daily(symbol="sh000001")
print(f"上证最后日期: {sh['date'].iloc[-1]}")
sh_latest = sh.iloc[-1]
print(f"上证成交额(手): {sh_latest['volume']}")

# 深证
sz = ak.stock_zh_index_daily(symbol="sz399001")
print(f"深证最后日期: {sz['date'].iloc[-1]}")
sz_latest = sz.iloc[-1]
print(f"深证成交额(手): {sz_latest['volume']}")

# 创业板
cy = ak.stock_zh_index_daily(symbol="sz399006")
print(f"创业板最后日期: {cy['date'].iloc[-1]}")
cy_latest = cy.iloc[-1]
print(f"创业板成交额(手): {cy_latest['volume']}")

# 注意akshare的volume单位是手，不是元
total = sh_latest['volume'] + sz_latest['volume'] + cy_latest['volume']
print(f"\n三市成交额(亿手): {total / 1e8:.2f}")

# 获取涨跌停统计
print("\n=== 涨跌停统计 ===")
df_zt = ak.stock_zt_pool_em(date="20260309")
print(f"涨停: {len(df_zt)}")

# 炸板股
df_zt_pool = ak.stock_zt_pool_strong_em(date="20260309")
print(f"炸板(含涨停): {len(df_zt_pool)}")
print(f"封板率: {len(df_zt)/len(df_zt_pool)*100:.1f}%")

# 跌停
df = ak.stock_zh_a_spot_em()
df_dt = df[df["涨跌幅"] <= -9.9]
print(f"跌停: {len(df_dt)}")
