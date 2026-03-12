import akshare as ak

# 看stock_zt_pool_em的参数
import inspect
sig = inspect.signature(ak.stock_zt_pool_em)
print("stock_zt_pool_em 参数:", sig)

# 试试pool类型
for pool in ["跌停", "炸板", "首板", "昨日涨停"]:
    try:
        df = ak.stock_zt_pool_em(date="20260309", pool=pool)
        print(f"\n{pool}: {len(df)}条")
    except Exception as e:
        print(f"{pool}: error - {e}")
