import requests
import json

# 东方财富实时接口
def get_stock_实时(code):
    """获取个股实时数据"""
    # secid: 0.深证 1.上海
    if code.startswith('6'):
        secid = f'1.{code}'
    else:
        secid = f'0.{code}'
    
    url = f'https://push2.eastmoney.com/api/qt/stock/get?secid={secid}&fields=f43,f44,f45,f46,f47,f48,f50,f51,f52,f57,f58,f59,f60,f116,f117,f118,f162,f167,f168,f169,f170,f171,f173,f177'
    
    try:
        resp = requests.get(url, timeout=5)
        data = resp.json()
        if data['rc'] == 0 and 'data' in data:
            d = data['data']
            # f43=最新价(单位:分), f173=涨跌幅(%)
            price = d.get('f43', 0) / 100
            pct = d.get('f173', 0)
            name = d.get('f58', '')
            pre_close = d.get('f60', 0) / 100
            return {'name': name, 'price': price, 'pct': pct, 'pre_close': pre_close}
    except Exception as e:
        print(f'Error: {e}')
        return None

# 验证昨天报告推荐的股票今日表现
print('=== 昨天(20260310)推荐股票今日(20260311)实时表现 ===')
print()
stocks = [
    ('605268', '王力安防', '空间板5连板'),
    ('600498', '烽火通信', '通信设备龙头'),
    ('601789', '宁波建工', '验证2-3连板'),
    ('688048', '长光华芯', '20cm弹性'),
]

results = []
for code, name, reason in stocks:
    result = get_stock_实时(code)
    if result:
        pct = result['pct']
        price = result['price']
        
        # 判断状态
        if pct >= 9.9:
            status = '✅ 涨停'
        elif pct >= 5:
            status = '📈 大涨'
        elif pct > 0:
            status = '⬆️ 上涨'
        elif pct > -5:
            status = '⬇️ 下跌'
        else:
            status = '💔 大跌'
        
        print(f'{name}({code})')
        print(f'  推荐理由: {reason}')
        print(f'  今日: {pct:+.2f}% ({price:.2f}元) {status}')
        
        results.append({'name': name, 'pct': pct, 'status': status})
    else:
        print(f'{name}({code}): 获取失败')
        results.append({'name': name, 'pct': 0, 'status': '获取失败'})
    print()

# 汇总
print('=' * 40)
print('验证结果汇总:')
for r in results:
    print(f"  {r['name']}: {r['pct']:+.2f}% {r['status']}")
