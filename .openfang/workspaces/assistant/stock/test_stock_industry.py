"""
验证：直接用东财接口获取股票的行业分类，能否和资金流向API匹配
"""
import requests
import json
import re

# 选一只涨停板的股票，看看东财接口返回什么行业
test_codes = ['002445', '603803', '002760']  # 通用设备的几个

headers = {"User-Agent": "Mozilla/5.0"}

for code in test_codes:
    # 东财个股详情接口
    url = f"https://push2.eastmoney.com/api/qt/stock/get"
    params = {
        "fltt": 2,
        "invt": 2,
        "fields": "f57,f58,f84,f85,f116,f117,f127,f128,f162,f163,f164,f167,f168,f169,f170,f171,f172,f173,f187,f188,f189,f190,f191,f192,f194,f195,f197,f198,f199,f200,f201,f202,f203,f204,f205,f206,f207,f208,f209,f210,f211,f212,f213,f214,f215,f216,f217,f218,f219,f220,f221,f222,f223,f224,f225,f226,f227,f228,f229,f230,f231,f232,f233,f234,f235,f236,f237,f238,f239,f240,f241,f242,f243,f244,f245,f246,f247,f248,f249,f250,f251,f252,f253,f254,f255,f256,f257,f258,f259,f260,f261,f262,f263,f264,f265,f266,f267,f268,f269,f270,f271,f272,f273,f274,f275,f276,f277,f278,f279,f280,f281,f282,f283,f284,f285,f286,f287,f288,f289,f290,f291,f292,f293,f294,f295,f296,f297,f298,f299,f300",
        "secid": f"1.{code}"  # 上海
    }
    
    try:
        resp = requests.get(url, params=params, headers=headers, timeout=10)
        data = resp.json()
        
        if data.get('data'):
            d = data['data']
            print(f"\n{code}:")
            # 行业分类相关字段
            print(f"  f57(名称): {d.get('f57')}")
            print(f"  f116(行业): {d.get('f116')}")
            print(f"  f117(行业): {d.get('f117')}")
            print(f"  f162(行业): {d.get('f162')}")
            print(f"  f163(行业): {d.get('f163')}")
    except Exception as e:
        print(f"{code}: {e}")
