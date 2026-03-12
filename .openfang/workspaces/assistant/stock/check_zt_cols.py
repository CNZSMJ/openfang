"""
akshare涨停板只有"所属行业"列
"""
import sys
sys.path.insert(0, '/Users/huangjiahao/.openfang/workspaces/assistant/stock')
import akshare as ak

df_zt = ak.stock_zt_pool_em(date='20260310')

print("=== akshare涨停板数据的列 ===")
print(df_zt.columns.tolist())

print("\n=== 涨停板行业分布 ===")
print(df_zt['所属行业'].value_counts().head(10))
