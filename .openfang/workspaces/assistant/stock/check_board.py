import akshare as ak

# 涨停板数据
df_zt = ak.stock_zt_pool_em(date='20260310')
print('=== 涨停板板块分布 ===')
print(df_zt['所属行业'].value_counts().head(10))

print('\n=== 涨停板中属于"通用设备"的股票 ===')
print(df_zt[df_zt['所属行业']=='通用设备'][['代码','名称','所属行业']].head(5))

print('\n=== 涨停板中属于"通信设备"的股票 ===')
print(df_zt[df_zt['所属行业']=='通信设备'][['代码','名称','所属行业']].head(5))
