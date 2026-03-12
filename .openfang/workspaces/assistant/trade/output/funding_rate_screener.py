#!/usr/bin/env python3
"""
资金费率套利筛选器
功能：从OKX获取所有永续合约资金费率，筛选最优品种
"""

import requests
import json
from datetime import datetime

# ================== 配置 ==================
OKX_API = "https://www.okx.com"
# 筛选阈值
MIN_FUNDING_RATE = 0.0001  # 最小资金费率 0.01%
MIN_VOLUME = 10_000_000   # 最小24h成交量 $1000万
TOP_N = 5                  # 显示前N个

# ================== 获取所有永续合约 ==================
def get_all_perpetual_instruments():
    """获取所有永续合约交易对"""
    url = f"{OKX_API}/api/v5/public/instruments"
    params = {
        "instType": "SWAP",
        "uly": ""  # 不过滤，全部获取
    }
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get("code") == "0":
            return [item["instId"] for item in data.get("data", [])]
        return []
    except Exception as e:
        print(f"获取合约列表失败: {e}")
        return []

# ================== 获取资金费率 ==================
def get_funding_rate(inst_id):
    """获取指定合约的资金费率"""
    url = f"{OKX_API}/api/v5/public/funding-rate"
    params = {"instId": inst_id}
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get("code") == "0" and data.get("data"):
            return data["data"][0]
        return None
    except:
        return None

# ================== 获取24h成交量 ==================
def get_24h_volume(inst_id):
    """获取24h成交量"""
    url = f"{OKX_API}/api/v5/market/ticker"
    params = {"instId": inst_id}
    try:
        resp = requests.get(url, params=params, timeout=10)
        data = resp.json()
        if data.get("code") == "0" and data.get("data"):
            item = data["data"][0]
            return {
                "vol24h": float(item.get("vol24h", 0)),       # 成交量（币）
                "volCcy24h": float(item.get("volCcy24h", 0)) # 成交量（美元）
            }
        return None
    except:
        return None

# ================== 计算年化收益率 ==================
def calc_annual_return(funding_rate):
    """
    计算年化收益率
    假设每天3次资金结算（8小时一次）
    """
    daily_rate = funding_rate * 3  # 每天
    annual_rate = daily_rate * 365 # 年化
    return annual_rate * 100       # 转为百分比

# ================== 主筛选逻辑 ==================
def screen_opportunities():
    print("🔍 正在获取OKX永续合约数据...\n")
    
    # 1. 获取所有合约
    instruments = get_all_perpetual_instruments()
    print(f"📊 共获取 {len(instruments)} 个永续合约\n")
    
    opportunities = []
    
    # 2. 逐个查询资金费率和成交量
    for i, inst_id in enumerate(instruments):
        # 只查询主流币种（USDT结算）
        if not inst_id.endswith("-USDT-SWAP"):
            continue
            
        # 进度显示
        if i % 50 == 0:
            print(f"   进度: {i}/{len(instruments)}")
        
        # 获取资金费率
        funding_data = get_funding_rate(inst_id)
        if not funding_data:
            continue
        
        # 解析费率
        try:
            funding_rate = float(funding_data.get("fundingRate", 0))
            next_funding_time = funding_data.get("nextFundingTime", "")
        except:
            continue
        
        # 获取成交量
        volume_data = get_24h_volume(inst_id)
        if not volume_data:
            continue
        
        vol_usd = volume_data["volCcy24h"]
        
        # 筛选条件
        if funding_rate >= MIN_FUNDING_RATE and vol_usd >= MIN_VOLUME:
            # 提取币种名
            symbol = inst_id.replace("-USDT-SWAP", "")
            
            opportunities.append({
                "symbol": symbol,
                "inst_id": inst_id,
                "funding_rate": funding_rate,
                "annual_return": calc_annual_return(funding_rate),
                "volume_usd": vol_usd,
                "next_funding": next_funding_time
            })
    
    # 3. 按年化收益率排序
    opportunities.sort(key=lambda x: x["annual_return"], reverse=True)
    
    return opportunities[:TOP_N]

# ================== 打印结果 ==================
def print_results(opportunities):
    print("\n" + "="*70)
    print("🎯 资金费率套利机会筛选结果")
    print("="*70)
    print(f"{'排名':<4} {'币种':<8} {'资金费率':<12} {'年化收益':<12} {'24h成交量':<15} {'下次结算'}")
    print("-"*70)
    
    for i, opp in enumerate(opportunities, 1):
        print(f"{i:<4} {opp['symbol']:<8} "
              f"{opp['funding_rate']*100:.4f}%     "
              f"{opp['annual_return']:.2f}%     "
              f"${opp['volume_usd']/1e6:.1f}M      "
              f"{opp['next_funding'][:16] if opp['next_funding'] else 'N/A'}")
    
    print("-"*70)
    
    # 推荐
    if opportunities:
        best = opportunities[0]
        print(f"\n🏆 推荐币种: {best['symbol']}")
        print(f"   预期年化收益: {best['annual_return']:.2f}%")
        print(f"   $10,000本金月收益: ${10000 * best['funding_rate'] * 3:.2f}")
        
        # 计算TOP3组合
        total_annual = sum(o['annual_return'] for o in opportunities[:3]) / 3
        print(f"\n📊 TOP3 组合平均年化: {total_annual:.2f}%")

# ================== 主程序 ==================
if __name__ == "__main__":
    print(f"⏰ 筛选时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print(f"📈 筛选条件: 资金费率 >= {MIN_FUNDING_RATE*100}%, 成交量 >= ${MIN_VOLUME/1e6}M\n")
    
    opportunities = screen_opportunities()
    print_results(opportunities)
