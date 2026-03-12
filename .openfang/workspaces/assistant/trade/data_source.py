# SS v3.0 回测系统 - 数据源配置

## 一、数据源架构

```
┌─────────────────────────────────────────────────────────────┐
│                      数据源层                                │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    │
│  │  Tushare    │    │  AkShare    │    │  本地CSV   │    │
│  │  (主数据源)  │    │  (备用)     │    │  (缓存)    │    │
│  └──────┬──────┘    └──────┬──────┘    └──────┬──────┘    │
│         │                  │                  │            │
│         └──────────────────┼──────────────────┘            │
│                            ↓                                 │
│                   ┌────────────────┐                        │
│                   │  DataManager   │                        │
│                   │  (数据统一接口) │                        │
│                   └────────┬───────┘                        │
│                            ↓                                 │
│  ┌──────────────────────────────────────────────────────┐  │
│  │                   数据存储层                           │  │
│  │  ├─ stock_daily (日线行情)                            │  │
│  │  ├─ stock_financial (财务数据)                         │  │
│  │  ├─ money_flow (资金流向)                              │  │
│  │  └─ stock_info (基本信息)                             │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

## 二、所需数据清单

| 数据类型 | 字段要求 | 来源 | 更新频率 |
|----------|----------|------|----------|
| 日线行情 | open, high, low, close, vol, amount | Tushare | 日更 |
| 财务数据 | pe_ttm, pb, roe, revenue_growth, gross_margin, cash_flow | Tushare | 季更 |
| 资金流向 | main_net_inflow, main_net_inflow_5d | Tushare | 日更 |
| 基本信息 | industry, market_cap, listed_date, holder_num | Tushare | 日更 |
| 限售股解禁 | restricted_shares | Tushare | 月更 |
| 减持计划 | reduction_plan | Tushare | 月更 |

## 三、Tushare 接口配置

```python
# 需要安装: pip install tushare
# Token 申请: https://tushare.pro/

TS_TOKEN = "your_token_here"  # 需要用户自行配置
```

## 四、备选数据源

如无 Tushare，可使用 AkShare（免费但速度较慢）:

```bash
pip install akshare
```

## 五、本地数据缓存

首次运行后数据会缓存到本地，避免重复请求:

```
data/
├── daily/
│   ├── 2024/
│   │   ├── 688008_2024.csv
│   │   └── ...
│   └── ...
├── financial/
│   └── financial_cache.csv
└── money_flow/
    └── money_flow_cache.csv
```
