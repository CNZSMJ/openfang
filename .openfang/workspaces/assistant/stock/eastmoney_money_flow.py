"""
东方财富板块资金流向API封装
直接调用，无需登录
"""
import requests
import json
import re


def get_board_money_flow(top_n=15):
    """
    获取东方财富板块资金流向（主力资金）
    
    参数:
        top_n: 返回前N个板块，默认15
        
    返回:
        list: [{'板块名': str, '主力净流入': float, '涨跌幅': float, ...}, ...]
    """
    url = "https://push2.eastmoney.com/api/qt/clist/get"
    params = {
        "cb": "jQuery112307879834664846898_1630941013041",
        "fid": "f62",  # 主力净流入
        "po": "1",     # 正序排列
        "pz": str(top_n),
        "pn": "1",
        "np": "1",
        "fltt": "2",
        "invt": "2",
        "ut": "b2884a393a59ad64002292a3e90d46a5",
        "fs": "m:90+t:2",  # 行业板块
        "fields": "f12,f14,f2,f3,f62,f184,f66,f69,f72,f75,f78,f81,f84,f87,f204,f205,f124,f1,f13"
    }
    
    headers = {
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"
    }
    
    try:
        resp = requests.get(url, params=params, headers=headers, timeout=10)
        text = resp.text
        
        # 提取JSONP回调里的JSON数据
        match = re.search(r'\{.*\}', text)
        if not match:
            return []
            
        data = json.loads(match.group(0))
        
        if data.get('data') and data['data'].get('diff'):
            results = []
            for item in data['data']['diff']:
                results.append({
                    '板块代码': item.get('f12', ''),
                    '板块名': item.get('f14', ''),
                    '最新价': item.get('f2', 0),
                    '涨跌幅': item.get('f3', 0),
                    '主力净流入': item.get('f62', 0),      # 主净流入(元)
                    '超大单净流入': item.get('f66', 0),     # 超大单净流入
                    '超大单净占比': item.get('f69', 0),    # 超大单净占比
                    '大单净流入': item.get('f72', 0),      # 大单净流入
                    '大单净占比': item.get('f75', 0),       # 大单净占比
                    '中单净流入': item.get('f78', 0),       # 中单净流入
                    '中单净占比': item.get('f81', 0),       # 中单净占比
                    '小单净流入': item.get('f84', 0),       # 小单净流入
                    '小单净占比': item.get('f87', 0),       # 小单净占比
                    '净流入额': item.get('f124', 0),        # 今日主力净流入额(元)
                    '主力净占比': item.get('f184', 0),      # 主力净占比
                    '主力净流入最大股': item.get('f204', ''),  # 主力净流入最大股
                    '主力净流入最大股代码': item.get('f205', ''),
                })
            return results
        return []
    except Exception as e:
        print(f"获取资金流向失败: {e}")
        return []


def format_money_flow_report(data, top_n=10):
    """
    格式化资金流向报告
    
    返回:
        str: markdown格式的报告
    """
    if not data:
        return "暂无数据"
    
    # 按主力净流入排序
    sorted_data = sorted(data, key=lambda x: x['主力净流入'] or 0, reverse=True)
    
    # 分为净流入和净流出
    inflow = [x for x in sorted_data if x['主力净流入'] > 0][:top_n]
    outflow = [x for x in sorted_data if x['主力净流入'] < 0][-top_n:][::-1]
    
    def fmt_money(v):
        if v is None:
            return "0"
        v = float(v)
        if abs(v) >= 1e8:
            return f"{v/1e8:.2f}亿"
        elif abs(v) >= 1e4:
            return f"{v/1e4:.2f}万"
        return f"{v:.0f}"
    
    report = "## 📊 板块资金流向（主力资金）\n\n"
    
    report += "### 🔥 净流入TOP10\n"
    report += "| 排名 | 板块 | 主力净流入 | 涨跌幅 | 主力净占比 | 净流入最大股 |\n"
    report += "|------|------|----------|--------|----------|------------|\n"
    
    for i, item in enumerate(inflow, 1):
        report += f"| {i} | {item['板块名']} | {fmt_money(item['主力净流入'])} | {item['涨跌幅']:+.2f}% | {item['主力净占比']:.2f}% | {item['主力净流入最大股']} |\n"
    
    report += "\n### 💔 净流出TOP10\n"
    report += "| 排名 | 板块 | 主力净流出 | 涨跌幅 | 主力净占比 |\n"
    report += "|------|------|----------|--------|----------|\n"
    
    for i, item in enumerate(outflow, 1):
        report += f"| {i} | {item['板块名']} | {fmt_money(item['主力净流入'])} | {item['涨跌幅']:+.2f}% | {item['主力净占比']:.2f}% |\n"
    
    return report


def get_hot_board_by_money(top_n=5):
    """
    获取资金流入最热的板块（简化版）
    
    返回:
        list: ['板块1', '板块2', ...]
    """
    data = get_board_money_flow(top_n)
    return [item['板块名'] for item in data if item['主力净流入'] > 0]


if __name__ == "__main__":
    # 测试
    print("=== 板块资金流向 ===")
    data = get_board_money_flow(15)
    print(format_money_flow_report(data))
