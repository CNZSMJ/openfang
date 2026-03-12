"""
涨停板短线选股策略 v2.0
基于五大维度：竞价、量价、形态、板块、首板强度
修复：收盘后使用涨跌幅排名数据
"""

import requests
import pandas as pd
from datetime import datetime
import json

# ==================== 东方财富数据接口 ====================
def get_涨跌幅排名(limit=200):
    """获取今日涨跌幅排名（从高到低）"""
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "pn": 1,
        "pz": limit,
        "po": 1,  # 降序
        "np": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f3",
        "fs": "m:0+t:6,m:0+t:80,m:1:t:2,m:1:t:23",
        "fields": "f2,f3,f4,f12,f13,f14,f15,f16,f17,f18,f20,f21,f24,f25,f37,f38,f39,f40,f41,f42,f43,f44,f45,f46,f47,f48,f49,f50,f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f62,f63,f64,f65,f66,f67,f68,f69,f70,f71,f72,f73,f74,f75,f76,f77,f78,f79,f80,f81,f82,f84,f85,f86,f87,f88,f89,f90,f91,f92,f93,f94,f95,f96,f97,f98,f99,f100,f101,f102,f103,f104,f105,f106,f107,f108,f109,f110,f111,f112,f113,f114,f115,f116,f117,f118,f119,f120,f121,f122,f123,f124,f125,f126,f127,f128,f129,f130,f131,f132,f133,f134,f135,f136,f137,f138,f139,f140,f141,f142,f143,f144,f145,f146,f147,f148,f149,f150,f151,f152,f153,f154,f155,f156,f157,f158,f159,f160,f161,f162,f163,f164,f165,f166,f167,f168,f169,f170,f171,f172,f173,f174,f175,f176,f177,f178,f179,f180,f181,f182,f183,f184,f185,f186,f187,f188,f189,f190,f191,f192,f193,f194,f195,f196,f197,f198,f199,f200,f201,f202,f203,f204,f205,f206,f207,f208,f209,f210,f211,f212,f213,f214,f215,f216,f217,f218,f219,f220,f221,f222,f223,f224,f225,f226,f227,f228,f229,f230,f231,f232,f233,f234,f235,f236,f237,f238,f239,f240,f241,f242,f243,f244,f245,f246,f247,f248,f249,f250,f251,f252,f253,f254,f255,f256,f257,f258,f259,f260,f261,f262,f263,f264,f265,f266,f267,f268,f269,f270,f271,f272,f273,f274,f275,f276,f277,f278,f279,f280,f281,f282,f283,f284,f285,f286,f287,f288,f289,f290,f291,f292,f293,f294,f295,f296,f297,f298,f299,f300,f301,f302,f303,f304,f305,f306,f307,f308,f309,f310,f311,f312,f313,f314,f315,f316,f317,f318,f319,f320,f321,f322,f323,f324,f325,f326,f327,f328,f329,f330,f331,f332,f333,f334,f335,f336,f337,f338,f339,f340,f341,f342,f343,f344,f345,f346,f347,f348,f349,f350,f351,f352,f353,f354,f355,f356,f357,f358,f359,f360,f361,f362,f363,f364,f365,f366,f367,f368,f369,f370,f371,f372,f373,f374,f375,f376,f377,f378,f379,f380,f381,f382,f383,f384,f385,f386,f387,f388,f389,f390,f391,f392,f393,f394,f395,f396,f397,f398,f399,f400,f401,f402,f403,f404,f405,f406,f407,f408,f409,f410,f411,f412,f413,f414,f415,f416,f417,f418,f419,f420,f421,f422,f423,f424,f425,f426,f427,f428,f429,f430,f431,f432,f433,f434,f435,f436,f437,f438,f439,f440,f441,f442,f443,f444,f445,f446,f447,f448,f449,f450,f451,f452,f453,f454,f455,f456,f457,f458,f459,f460,f461,f462,f463,f464,f465,f466,f467,f468,f469,f470,f471,f472,f473,f474,f475,f476,f477,f478,f479,f480,f481,f482,f483,f484,f485,f486,f487,f488,f489,f490,f491,f492,f493,f494,f495,f496,f497,f498,f499,f500",
        "_t": datetime.now().timestamp()
    }
    
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get('data') and data['data'].get('diff'):
            df = pd.DataFrame(data['data']['diff'])
            return df
    except Exception as e:
        print(f"获取涨跌幅排名失败: {e}")
    return pd.DataFrame()

def get_板块涨跌():
    """获取板块涨跌排名"""
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "pn": 1,
        "pz": 30,
        "po": 1,
        "np": 1,
        "fltt": 2,
        "invt": 2,
        "fid": "f3",
        "fs": "b:MK0021,b:MK0022,b:MK0023,b:MK0024",  # 行业板块
        "fields": "f2,f3,f4,f12,f13,f14,f15,f16,f17,f18",
        "_t": datetime.now().timestamp()
    }
    
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get('data') and data['data'].get('diff'):
            df = pd.DataFrame(data['data']['diff'])
            return df.sort_values('f3', ascending=False).head(20)
    except Exception as e:
        print(f"获取板块失败: {e}")
    return pd.DataFrame()

# ==================== 选股策略核心 ====================
def 评估个股强度(row):
    """
    评估个股强度（基于五大维度）
    返回: (得分, 评级, 原因)
    """
    score = 0
    reasons = []
    
    # 维度1: 涨跌幅 (权重30%)
    # 越接近涨停越好
    zdf = row.get('f3', 0) or 0
    if zdf >= 9.9:
        score += 30
        reasons.append("涨停板")
    elif zdf >= 8:
        score += 20
        reasons.append(f"接近涨停({zdf:.1f}%)")
    elif zdf >= 5:
        score += 10
        reasons.append(f"大涨({zdf:.1f}%)")
    
    # 维度2: 换手率 (权重20%)
    hss = row.get('f37', 0) or 0
    if hss:
        if 5 <= hss <= 30:
            score += 20
            reasons.append(f"换手率{hss:.1f}%适中")
        elif hss < 5:
            score += 10
            reasons.append(f"换手率{hss:.1f}%偏低")
        elif hss > 30:
            score -= 10
            reasons.append(f"换手率{hss:.1f}%过高")
    
    # 维度3: 封单金额 (权重20%)
    fdm = row.get('f84', 0) or row.get('f122', 0) or 0
    if fdm:
        if fdm >= 50000000:
            score += 20
            reasons.append(f"封单{fdm/10000:.0f}万")
        elif fdm >= 10000000:
            score += 12
            reasons.append(f"封单{fdm/10000:.0f}万")
        elif fdm > 0:
            score += 5
    
    # 维度4: 流通市值 (权重15%)
    ltsz = row.get('f20', 0) or 0
    if ltsz:
        sz = ltsz / 100000000
        if 50 <= sz <= 300:
            score += 15
            reasons.append(f"市值{sz:.0f}亿适中")
        elif sz < 50:
            score += 10
            reasons.append(f"小市值{sz:.0f}亿弹性大")
        else:
            score += 5
    
    # 维度5: 量比 (权重15%)
    lb = row.get('f38', 0) or 0
    if lb:
        if lb >= 1.5:
            score += 15
            reasons.append(f"量比{lb:.1f}活跃")
        elif lb >= 1:
            score += 10
        else:
            score += 5
    
    # 评级
    if score >= 85:
        rating = "S级-必打"
    elif score >= 70:
        rating = "A级-重点"
    elif score >= 55:
        rating = "B级-可打"
    elif score >= 40:
        rating = "C级-观察"
    else:
        rating = "D级-放弃"
    
    return score, rating, "; ".join(reasons[:3])

def 选股策略():
    """执行选股策略"""
    print("=" * 60)
    print(f"【涨停板短线选股策略 v2.0】运行时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 60)
    
    # 1. 获取涨跌幅排名
    print("\n[1/3] 获取涨跌幅排名...")
    df = get_涨跌幅排名(300)
    if df.empty:
        print("获取数据失败！")
        return
    print(f"获取到 {len(df)} 只股票")
    
    # 2. 筛选涨幅>5%的
    zt_df = df[df['f3'] >= 5].copy()
    print(f"涨幅>5%的股票: {len(zt_df)} 只")
    
    # 3. 获取热门板块
    print("\n[2/3] 获取热门板块...")
    hot_bk = get_板块涨跌()
    if not hot_bk.empty:
        print("今日涨幅前5板块:")
        for _, row in hot_bk.head(5).iterrows():
            print(f"  {row['f14']}: {row['f3']:+.2f}%")
    
    # 4. 评估每只股票
    print("\n[3/3] 评估个股强度...")
    results = []
    for idx, row in zt_df.iterrows():
        code = str(row.get('f12', ''))
        name = row.get('f14', '')
        
        if not code or not name:
            continue
        
        # 评估强度
        score, rating, reasons = 评估个股强度(row)
        
        results.append({
            'code': code,
            'name': name,
            'price': row.get('f2', 0),
            'pct': row.get('f3', 0),
            'hss': row.get('f37', 0),
            'lb': row.get('f38', 0),
            'ltsz': row.get('f20', 0),
            'score': score,
            'rating': rating,
            'reasons': reasons
        })
    
    # 5. 排序输出
    print("\n" + "=" * 60)
    print("选股结果:")
    print("=" * 60)
    
    results_df = pd.DataFrame(results)
    if results_df.empty:
        print("无符合条件的股票")
        return
    
    results_df = results_df.sort_values('score', ascending=False)
    
    # 显示Top 15
    print("\n【S级 - 必打】(分数>=85)")
    s_df = results_df[results_df['score'] >= 85]
    for _, r in s_df.head(10).iterrows():
        print(f"  ★ {r['name']}({r['code']}) {r['pct']:+.2f}% 换手:{r['hss']:.1f}% 量比:{r['lb']:.1f}")
        print(f"     理由: {r['reasons']}")
    
    print("\n【A级 - 重点】(70-84)")
    a_df = results_df[(results_df['score'] >= 70) & (results_df['score'] < 85)]
    for _, r in a_df.head(10).iterrows():
        print(f"  ◆ {r['name']}({r['code']}) {r['pct']:+.2f}% 分数:{r['score']}")
    
    print("\n【B级 - 可打】(55-69)")
    b_df = results_df[(results_df['score'] >= 55) & (results_df['score'] < 70)]
    for _, r in b_df.head(5).iterrows():
        print(f"  ● {r['name']}({r['code']}) {r['pct']:+.2f}% 分数:{r['score']}")
    
    print(f"\n总计: S级{len(s_df)}只，A级{len(a_df)}只，B级{len(b_df)}只")
    
    # 保存结果
    results_df.to_csv(f'选股结果_{datetime.now().strftime("%Y%m%d")}.csv', index=False, encoding='utf-8-sig')
    print(f"\n结果已保存")

if __name__ == "__main__":
    选股策略()
