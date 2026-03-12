import akshare as ak

# 搜索涨跌比相关的接口
stock_em = [x for x in dir(ak) if 'stock' in x and 'em' in x]

# 打印所有包含某些关键词的
keywords = ['涨跌', '上涨', '下跌', '统计', 'daily', 'overview']
for kw in keywords:
    matches = [x for x in stock_em if kw in x]
    if matches:
        print(f"\n=== {kw} ===")
        print(matches[:10])
