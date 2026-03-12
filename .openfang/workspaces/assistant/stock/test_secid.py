"""
修正：正确的secid格式
"""
import requests

headers = {"User-Agent": "Mozilla/5.0"}

# 测试不同股票
test_stocks = [
    ('002445', '深圳'),
    ('603803', '上海'),
    ('000001', '深圳'),
    ('600000', '上海'),
]

for code, market in test_stocks:
    # 根据市场选择前缀
    prefix = '0' if market == '深圳' else '1'
    
    url = "https://push2.eastmoney.com/api/qt/stock/get"
    params = {
        "fltt": 2,
        "fields": "f100,f14",
        "secid": f"{prefix}.{code}"
    }
    
    resp = requests.get(url, params=params, headers=headers, timeout=5)
    data = resp.json()
    
    if data.get('data'):
        print(f"{code}({market}): f100={data['data'].get('f100')}, name={data['data'].get('f14')}")
