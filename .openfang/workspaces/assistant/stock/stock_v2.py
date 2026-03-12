"""
涨停板短线选股策略 v2.0
简化版API调用
"""

import requests
import pandas as pd
from datetime import datetime

def get_涨跌幅排名(limit=200):
    """获取今日涨跌幅排名"""
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "pn": 1,
        "pz": limit,
        "po": 1,
        "np": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f3",
        "fs": "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23",
        "fields": "f2,f3,f4,f12,f14,f20,f37,f38,f45,f46"
    }
    
    try:
        resp = requests.get(url, params=params, timeout=15)
        data = resp.json()
        if data.get('data') and data['data'].get('diff'):
            df = pd.DataFrame(data['data']['diff'])
            return df
    except Exception as e:
        print(f"获取失败: {e}")
    return pd.DataFrame()

def get_板块涨跌():
    """获取板块涨跌"""
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "pn": 1,
        "pz": 20,
        "po": 1,
        "np": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f3",
        "fs": "b:MK0021,b:MK0022,b:MK0023,b:MK0024",
        "fields": "f2,f3,f4,f12,f14"
    }
    
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get('data') and data['data'].get('diff'):
            df = pd.DataFrame(data['data']['diff'])
            return df.sort_values('f3', ascending=False)
    except Exception as e:
        print(f"获取板块失败: {e}")
    return pd.DataFrame()

def 评估个股(row):
    """评估个股强度"""
    score = 0
    reasons = []
    
    # 涨跌幅
    zdf = row.get('f3', 0) or 0
    if zdf >= 9.9:
        score += 30
        reasons.append("涨停板")
    elif zdf >= 8:
        score += 20
        reasons.append(f"接近涨停({zdf:.1f}%)")
    elif zdf >= 5:
        score += 10
    
    # 换手率
    hss = row.get('f37', 0) or 0
    if 5 <= hss <= 30:
        score += 20
        reasons.append(f"换手{hss:.1f}%")
    elif hss < 5:
        score += 10
    elif hss > 30:
        score -= 10
    
    # 量比
    lb = row.get('f38', 0) or 0
    if lb >= 1.5:
        score += 15
        reasons.append(f"量比{lb:.1f}")
    elif lb >= 1:
        score += 10
    
    # 市值
    ltsz = row.get('f20', 0) or 0
    if ltsz:
        sz = ltsz / 100000000
        if sz <= 100:
            score += 15
            reasons.append(f"小市值{sz:.0f}亿")
        elif sz <= 300:
            score += 10
    
    # 评级
    if score >= 80:
        rating = "S级"
    elif score >= 65:
        rating = "A级"
    elif score >= 50:
        rating = "B级"
    else:
        rating = "C级"
    
    return score, rating, "; ".join(reasons[:2])

def 选股():
    print("=" * 60)
    print(f"【涨停板选股策略v2.0】 {datetime.now().strftime('%Y-%m-%d %H:%M')}")
    print("=" * 60)
    
    # 获取数据
    df = get_涨跌幅排名(300)
    if df.empty:
        print("获取数据失败")
        return
    
    print(f"\n获取到 {len(df)} 只股票")
    
    # 筛选涨幅>5%
    df = df[df['f3'] >= 5].copy()
    print(f"涨幅>5%: {len(df)} 只")
    
    # 板块
    print("\n--- 热门板块 ---")
    bk = get_板块涨跌()
    if not bk.empty:
        for _, r in bk.head(5).iterrows():
            print(f"  {r['f14']}: {r['f3']:+.2f}%")
    
    # 评估
    print("\n--- 选股结果 ---")
    results = []
    for _, row in df.iterrows():
        code = str(row.get('f12', ''))
        name = row.get('f14', '')
        if not code:
            continue
        
        score, rating, reasons = 评估个股(row)
        results.append({
            'code': code,
            'name': name,
            'price': row.get('f2', 0),
            'pct': row.get('f3', 0),
            'hss': row.get('f37', 0),
            'lb': row.get('f38', 0),
            'score': score,
            'rating': rating,
            'reasons': reasons
        })
    
    results_df = pd.DataFrame(results).sort_values('score', ascending=False)
    
    # 输出
    s_df = results_df[results_df['score'] >= 80]
    a_df = results_df[(results_df['score'] >= 65) & (results_df['score'] < 80)]
    
    print(f"\n【S级】{len(s_df)}只")
    for _, r in s_df.head(10).iterrows():
        print(f"  ★{r['name']}({r['code']}) {r['pct']:+.2f}% 换{r['hss']:.1f}% 量{r['lb']:.1f}")
        print(f"    {r['reasons']}")
    
    print(f"\n【A级】{len(a_df)}只")
    for _, r in a_df.head(10).iterrows():
        print(f"  ◆{r['name']}({r['code']}) {r['pct']:+.2f}% 分{r['score']}")
    
    # 保存
    results_df.to_csv(f'选股结果_{datetime.now().strftime("%Y%m%d")}.csv', index=False, encoding='utf-8-sig')
    print(f"\n已保存")

if __name__ == "__main__":
    选股()
