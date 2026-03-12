"""
测试东财不同板块分类API
"""
import requests
import json
import re

# 测试不同的fs参数
test_configs = [
    ("行业板块(90+t:2)", "m:90+t:2"),
    ("行业板块(90+t:1)", "m:90+t:1"),
    ("概念板块(90+t:3)", "m:90+t:3"),
]

headers = {"User-Agent": "Mozilla/5.0"}

for name, fs in test_configs:
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "pn": 1,
        "pz": 50,
        "po": 1,
        "np": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f62",
        "fs": fs,
        "fields": "f12,f14,f2,f3,f62"
    }
    
    try:
        resp = requests.get(url, params=params, headers=headers, timeout=10)
        text = resp.text
        match = re.search(r'\{.*\}', text)
        if match:
            data = json.loads(match.group(0))
            if data.get('data') and data['data'].get('diff'):
                boards = [item['f14'] for item in data['data']['diff']]
                print(f"\n=== {name} ===")
                print(boards[:20])
    except Exception as e:
        print(f"{name}: {e}")
