#!/usr/bin/env python3
"""Fetches the symbol logos that ship inside Opentern.

Writes crates/app/assets/logos/<key>.logo (the format logos.rs keeps) and
crates/app/src/logos_bundled.rs (the list, as include_str!). Run it again to
refresh or extend the set; the app fetches anything else at runtime.

Sources (both MIT; the marks themselves stay their owners' trademarks):
- Coins: web3icons (@web3icons/core), the "background" token icons: the
  brand's colour or gradient with the mark in white, cropped round in the app.
- Companies: theSVG (@thesvg/icons), the one-colour "mono" mark, drawn white
  on the brand colour in the app (or the full mark on white when there's no
  mono version).

    python3 scripts/fetch-logos.py
"""

import json
import pathlib
import re
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "crates" / "app" / "assets" / "logos"
LIST = ROOT / "crates" / "app" / "src" / "logos_bundled.rs"
W3 = "@web3icons/core@4.0.58"
THESVG = "@thesvg/icons@3.3.14"
MIRRORS = ["https://unpkg.com/", "https://cdn.jsdelivr.net/npm/"]

COINS = """BTC ETH SOL BNB XRP DOGE ADA AVAX TRX LINK DOT POL MATIC TON SHIB LTC BCH NEAR UNI APT ICP XLM ETC
FIL ATOM ARB OP HBAR VET INJ IMX SUI SEI TIA RENDER GRT AAVE MKR ALGO EGLD SAND MANA AXS THETA FLOW XTZ
EOS KAS PEPE WIF BONK FLOKI JUP PYTH ENA ONDO WLD STX RUNE LDO CRV SNX COMP CAKE GALA CHZ APE DYDX GMX
ZEC DASH XMR NEO IOTA KSM ZIL ENJ BAT 1INCH SUSHI YFI CELO QNT FET AGIX OCEAN TAO ORDI JTO STRK ZK
BLUR MEME NOT TRUMP BOME PENDLE ETHFI W ENS RPL SSV FXS LRC MASK API3 BAND KAVA ROSE ONE ICX ANKR SKL
USDT USDC DAI FDUSD TUSD WBTC WETH STETH HYPE BERA S FTM""".split()

# (exchange, ticker, company name, theSVG slug if the name doesn't lead to it)
COMPANIES = [
    ("NASDAQ", "AAPL", "Apple", "apple"), ("NASDAQ", "MSFT", "Microsoft", "microsoft"),
    ("NASDAQ", "NVDA", "NVIDIA", "nvidia"), ("NASDAQ", "AMZN", "Amazon", "amazon"),
    ("NASDAQ", "GOOGL", "Google", "google"), ("NASDAQ", "GOOG", "Google", "google"),
    ("NASDAQ", "META", "Meta", "meta"), ("NASDAQ", "TSLA", "Tesla", "tesla"),
    ("NASDAQ", "AVGO", "Broadcom", "broadcom"), ("NASDAQ", "NFLX", "Netflix", "netflix"),
    ("NASDAQ", "AMD", "AMD", "amd"), ("NASDAQ", "INTC", "Intel", "intel"),
    ("NASDAQ", "ADBE", "Adobe", "adobe"), ("NASDAQ", "CSCO", "Cisco", "cisco"),
    ("NASDAQ", "PEP", "PepsiCo", "pepsi"), ("NASDAQ", "COST", "Costco", "costco"),
    ("NASDAQ", "QCOM", "Qualcomm", "qualcomm"), ("NASDAQ", "TXN", "Texas Instruments", "texas-instruments"),
    ("NASDAQ", "INTU", "Intuit", "intuit"), ("NASDAQ", "PYPL", "PayPal", "paypal"),
    ("NASDAQ", "SBUX", "Starbucks", "starbucks"), ("NASDAQ", "ABNB", "Airbnb", "airbnb"),
    ("NASDAQ", "PLTR", "Palantir", "palantir"), ("NASDAQ", "MU", "Micron", "micron-technology"),
    ("NASDAQ", "ARM", "Arm", "arm"), ("NASDAQ", "COIN", "Coinbase", "coinbase"),
    ("NASDAQ", "MSTR", "MicroStrategy", "strategy"), ("NASDAQ", "SHOP", "Shopify", "shopify"),
    ("NASDAQ", "BKNG", "Booking.com", "booking-com"), ("NASDAQ", "MRNA", "Moderna", "moderna"),
    ("NASDAQ", "ZM", "Zoom", "zoom"), ("NASDAQ", "DDOG", "Datadog", "datadog"),
    ("NASDAQ", "CRWD", "CrowdStrike", "crowdstrike"), ("NASDAQ", "PANW", "Palo Alto Networks", "palo-alto-networks"),
    ("NASDAQ", "TEAM", "Atlassian", "atlassian"), ("NASDAQ", "WDAY", "Workday", "workday"),
    ("NASDAQ", "ASML", "ASML", "asml"), ("NASDAQ", "MELI", "Mercado Libre", "mercadolibre"),
    ("NASDAQ", "PDD", "Temu", "temu"), ("NASDAQ", "EA", "Electronic Arts", "ea"),
    ("NASDAQ", "MAR", "Marriott", "marriott"), ("NASDAQ", "HOOD", "Robinhood", "robinhood"),
    ("NYSE", "BRK.B", "Berkshire Hathaway", "berkshire-hathaway"), ("NYSE", "JPM", "JPMorgan Chase", "jpmorgan-chase"),
    ("NYSE", "V", "Visa", "visa"), ("NYSE", "MA", "Mastercard", "mastercard"),
    ("NYSE", "WMT", "Walmart", "walmart"), ("NYSE", "JNJ", "Johnson & Johnson", "johnson-and-johnson"),
    ("NYSE", "PG", "Procter & Gamble", "procter-and-gamble"), ("NYSE", "XOM", "ExxonMobil", "exxonmobil"),
    ("NYSE", "CVX", "Chevron", "chevron"), ("NYSE", "KO", "Coca-Cola", "coca-cola"),
    ("NYSE", "DIS", "Disney", "disney"), ("NYSE", "MCD", "McDonald's", "mcdonalds"),
    ("NYSE", "NKE", "Nike", "nike"), ("NYSE", "ORCL", "Oracle", "oracle"),
    ("NYSE", "CRM", "Salesforce", "salesforce"), ("NYSE", "IBM", "IBM", "ibm"),
    ("NYSE", "BA", "Boeing", "boeing"), ("NYSE", "GS", "Goldman Sachs", "goldman-sachs"),
    ("NYSE", "MS", "Morgan Stanley", "morgan-stanley"), ("NYSE", "BAC", "Bank of America", "bank-of-america"),
    ("NYSE", "WFC", "Wells Fargo", "wells-fargo"), ("NYSE", "C", "Citi", "citi"),
    ("NYSE", "AXP", "American Express", "american-express"), ("NYSE", "UNH", "UnitedHealth", "unitedhealth-group"),
    ("NYSE", "PFE", "Pfizer", "pfizer"), ("NYSE", "LLY", "Eli Lilly", "eli-lilly"),
    ("NYSE", "MRK", "Merck", "merck"), ("NYSE", "ABBV", "AbbVie", "abbvie"),
    ("NYSE", "T", "AT&T", "at-and-t"), ("NYSE", "VZ", "Verizon", "verizon"),
    ("NYSE", "HD", "Home Depot", "home-depot"), ("NYSE", "UBER", "Uber", "uber"),
    ("NYSE", "SPOT", "Spotify", "spotify"), ("NYSE", "SNOW", "Snowflake", "snowflake"),
    ("NYSE", "NOW", "ServiceNow", "servicenow"), ("NYSE", "TSM", "TSMC", "tsmc"),
    ("NYSE", "BABA", "Alibaba", "alibaba"), ("NYSE", "SONY", "Sony", "sony"),
    ("NYSE", "TM", "Toyota", "toyota"), ("NYSE", "GE", "General Electric", "general-electric"),
    ("NYSE", "F", "Ford", "ford"), ("NYSE", "GM", "General Motors", "general-motors"),
    ("NYSE", "CAT", "Caterpillar", "caterpillar"), ("NYSE", "UPS", "UPS", "ups"),
    ("NYSE", "FDX", "FedEx", "fedex"), ("NYSE", "LMT", "Lockheed Martin", "lockheed-martin"),
    ("NYSE", "SHEL", "Shell", "shell"), ("NYSE", "BP", "BP", "bp"),
    ("NYSE", "NVO", "Novo Nordisk", "novo-nordisk"), ("NYSE", "SAP", "SAP", "sap"),
    ("NYSE", "ACN", "Accenture", "accenture"), ("NYSE", "SNAP", "Snap", "snapchat"),
    ("NYSE", "RBLX", "Roblox", "roblox"), ("NYSE", "DELL", "Dell", "dell"),
    ("NYSE", "HPQ", "HP", "hp"), ("NYSE", "PINS", "Pinterest", "pinterest"),
    ("NSE", "RELIANCE", "Reliance Industries", "reliance-industries-limited"),
    ("NSE", "TCS", "Tata Consultancy Services", "tata-consultancy-services"),
    ("NSE", "HDFCBANK", "HDFC Bank", "hdfc-bank"), ("NSE", "ICICIBANK", "ICICI Bank", "icici-bank"),
    ("NSE", "INFY", "Infosys", "infosys"), ("NSE", "SBIN", "State Bank of India", "state-bank-of-india"),
    ("NSE", "BHARTIARTL", "Airtel", "airtel"), ("NSE", "ITC", "ITC", "itc"),
    ("NSE", "HINDUNILVR", "Hindustan Unilever", "unilever"), ("NSE", "LT", "Larsen & Toubro", "larsen-and-toubro"),
    ("NSE", "KOTAKBANK", "Kotak Mahindra Bank", "kotak-mahindra-bank"), ("NSE", "AXISBANK", "Axis Bank", "axis-bank"),
    ("NSE", "BAJFINANCE", "Bajaj Finance", "bajaj-finserv"), ("NSE", "MARUTI", "Maruti Suzuki", "maruti-suzuki"),
    ("NSE", "SUNPHARMA", "Sun Pharma", "sun-pharma"), ("NSE", "TATAMOTORS", "Tata Motors", "tata"),
    ("NSE", "TATASTEEL", "Tata Steel", "tata"), ("NSE", "TITAN", "Titan", "titan-company"),
    ("NSE", "WIPRO", "Wipro", "wipro"), ("NSE", "HCLTECH", "HCLTech", "hcl"),
    ("NSE", "TECHM", "Tech Mahindra", "tech-mahindra"), ("NSE", "ASIANPAINT", "Asian Paints", "asian-paints"),
    ("NSE", "ULTRACEMCO", "UltraTech Cement", "ultratech-cement"), ("NSE", "NESTLEIND", "Nestle", "nestle"),
    ("NSE", "M&M", "Mahindra", "mahindra"), ("NSE", "ADANIENT", "Adani", "adani"),
    ("NSE", "ADANIPORTS", "Adani Ports", "adani"), ("NSE", "ONGC", "ONGC", "ongc"),
    ("NSE", "NTPC", "NTPC", "ntpc"), ("NSE", "POWERGRID", "Power Grid", "power-grid"),
    ("NSE", "COALINDIA", "Coal India", "coal-india"), ("NSE", "JSWSTEEL", "JSW", "jsw"),
    ("NSE", "HEROMOTOCO", "Hero MotoCorp", "hero-motocorp"), ("NSE", "BAJAJ-AUTO", "Bajaj Auto", "bajaj-auto"),
    ("NSE", "EICHERMOT", "Royal Enfield", "royal-enfield"), ("NSE", "DRREDDY", "Dr. Reddy's", "dr-reddys"),
    ("NSE", "CIPLA", "Cipla", "cipla"), ("NSE", "APOLLOHOSP", "Apollo Hospitals", "apollo-hospitals"),
    ("NSE", "ZOMATO", "Zomato", "zomato"), ("NSE", "ETERNAL", "Zomato", "zomato"),
    ("NSE", "PAYTM", "Paytm", "paytm"), ("NSE", "NYKAA", "Nykaa", "nykaa"),
    ("NSE", "IRCTC", "IRCTC", "irctc"), ("NSE", "INDIGO", "IndiGo", "indigo"),
    ("NSE", "DMART", "DMart", "dmart"), ("NSE", "TATAPOWER", "Tata Power", "tata"),
    ("NSE", "IOC", "Indian Oil", "indian-oil"), ("NSE", "BPCL", "Bharat Petroleum", "bharat-petroleum"),
    ("NSE", "HAL", "Hindustan Aeronautics", "hal"), ("NSE", "BEL", "Bharat Electronics", "bel"),
    ("NSE", "MRF", "MRF", "mrf"), ("NSE", "PIDILITIND", "Pidilite", "pidilite"),
]


def get(path):
    for base in MIRRORS:
        try:
            with urllib.request.urlopen(base + path, timeout=20) as r:
                if r.status == 200:
                    return r.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
        except Exception:
            continue
    return None


def js_string(src):
    """The string in `var X = '...'` (or "..." / `...`), unescaped."""
    m = re.search(r"=\s*(['\"`])((?:\\.|(?!\1).)*)\1", src, re.S)
    if not m:
        return None
    raw = m.group(2)
    return raw.encode("latin-1", "backslashreplace").decode("unicode_escape")


def thesvg(slug):
    js = get(f"{THESVG}/dist/{slug}.js")
    if not js:
        return None
    hexm = re.search(r'export const hex = "([^"]*)"', js)
    hex_ = hexm.group(1) if hexm else ""
    mono = re.search(r'"mono": `([^`]*)`', js)
    if mono:
        return f"mark\n{hex_}\nmono\n{mono.group(1)}"
    full = re.search(r"export const svg = `([^`]*)`", js)
    return f"mark\n{hex_}\ndefault\n{full.group(1)}" if full else None


def slugs():
    with urllib.request.urlopen(f"https://data.jsdelivr.com/v1/package/npm/{THESVG}/flat", timeout=30) as r:
        files = json.load(r)["files"]
    return {f["name"][6:-3] for f in files if f["name"].startswith("/dist/") and f["name"].endswith(".js")}


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    known = slugs()
    kept = {}
    for coin in COINS:
        src = get(f"{W3}/dist/svgs/tokens/background/{coin}.svg.js")
        svg = src and js_string(src)
        if svg and "<svg" in svg:
            kept[f"coin-{coin.lower()}"] = f"square\n{svg}"
        else:
            print(f"  no coin logo: {coin}", file=sys.stderr)
    for ex, ticker, name, slug in COMPANIES:
        t = "".join(c for c in ticker if c.isalnum()).lower()
        key = f"co-{ex.lower()}-{t}"
        if slug not in known:
            print(f"  not in theSVG: {slug} ({ticker})", file=sys.stderr)
            continue
        logo = thesvg(slug)
        if logo:
            kept[key] = logo
    for old in OUT.glob("*.logo"):
        old.unlink()
    for key, body in sorted(kept.items()):
        (OUT / f"{key}.logo").write_text(body)
    lines = [
        "// Generated by scripts/fetch-logos.py: the logos that ship with the app.",
        "// Sources and licences: crates/app/assets/logos/README.md.",
        "",
        "pub const BUNDLED: &[(&str, &str)] = &[",
    ]
    for key in sorted(kept):
        lines.append(f'    ("{key}", include_str!("../assets/logos/{key}.logo")),')
    lines += ["];", ""]
    LIST.write_text("\n".join(lines))
    print(f"wrote {len(kept)} logos")


if __name__ == "__main__":
    main()
