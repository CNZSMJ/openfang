import akshare as ak
import pandas as pd

# 看涨停统计列的内容
df = ak.stock_zt_pool_em(date="20260309")
print("涨停统计列内容:")
print(df['涨停统计'].head(10))

# 看看炸板股的特征
# 炸板股：炸板次数 > 0
df['炸板次数_num'] = pd.to_numeric(df['炸板次数'], errors='coerce').fillna(0).astype(int)
zhaban = df[df['炸板次数_num'] > 0]
print(f"\n炸板股数量: {len(zhaban)}")
print(zhaban[['名称', '代码', '涨跌幅', '炸板次数', '涨停统计']].head(5))

# 搜索涨跌停统计接口
print("\n\n搜索相关接口...")
zt_related = [name for name in dir(ak) if ('zt' in name or 'limit' in name) and 'em' in name]
print(zt_related)
