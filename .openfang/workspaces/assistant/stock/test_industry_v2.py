"""
用东财行情中心接口获取股票行业
"""
import requests
import json

headers = {"User-Agent": "Mozilla/5.0"}

# 试试不同接口
test_codes = ['002445', '603803']

for code in test_codes:
    # 接口1
    url = f"https://quote.eastmoney.com/sh{code}.html"
    
    # 接口2: 股票列表接口
    url2 = "https://24k.cn/api/quotes/stock/_hd"
    params2 = {"symbol": code}
    
    # 接口3: 东财免费接口
    url3 = "https://push2ex.eastmoney.com/getTopicZDFenBu"
    params3 = {"ut": "7eea3edcaed734bea9cbfc24409ed989", "dession": "ALL", "mession": "ALL"}
    
    # 接口4: 获取个股所属板块
    url4 = "https://emweb.securities.eastmoney.com/PC_HSF10/NewFinanceAnalysis/Index?type=web&code="
    
    # 接口5: 资金流向接口 - 包含所属板块
    url5 = "https://push2.eastmoney.com/api/qt/stock/get"
    params5 = {
        "fltt": 2,
        "fields": "f57,f58,f84,f85,f116,f117,f127,f128,f162,f163,f164,f167,f168,f169,f170,f171,f172,f173,f187,f188,f189,f190,f191,f192,f194,f195,f197,f198,f199,f200,f201,f202,f203,f204,f205,f206,f207,f208,f209,f210",
        "secid": f"0.{code}"
    }
    
    try:
        resp = requests.get(url5, params=params5, headers=headers, timeout=5)
        data = resp.json()
        if data.get('data'):
            d = data['data']
            print(f"\n{code}:")
            # 打印所有非空字段
            for k, v in d.items():
                if v and str(v) != '0' and str(v) != '0.0':
                    print(f"  {k}: {v}")
    except Exception as e:
        print(f"{code}: {e}")
