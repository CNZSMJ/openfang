import akshare as ak
stock_em = [x for x in dir(ak) if 'stock' in x and 'em' in x]
print(stock_em[:30])
