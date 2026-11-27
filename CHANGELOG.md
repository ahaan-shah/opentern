# Changelog

## Unreleased

### Fixed
- **Hyperliquid prices keep the right decimals as they move.** Hyperliquid's price step depends on the price (five significant figures), so a coin that crosses 1, 10 or 1,000 (or BTC below 10,000) now shows and takes one decimal more or less right away, on the chart, in watchlists and in the DOM.

## 0.3.8 (2026-11-28)

Hyperliquid's markets, a deeper equity chart in the Strategy Report, Fib levels and drawing templates you choose, the Fed funds rate, indicators on indicators, and strategies that recalculate on order fills.

### New
- **The Fed funds rate.** Search "fed funds" or "EFFR" for the US effective federal funds rate, daily since July 2000, from the Federal Reserve Bank of New York. The Fed's target range shows as a band behind it and in the legend ("Target 3.75–4.00%"), and the day's change is in basis points. It's published once a day around 9:00 a.m. New York time, so it has daily, weekly and monthly charts only.
- **Correlation Coefficient** is a built-in: how closely the chart moves with another symbol over the last bars, from −1 to 1. Adding it opens the symbol search to pick that symbol.
- **Symbol inputs use the symbol search.** A script's `input.symbol` is picked like any symbol, from part of its name; with `confirm = true` the search opens as soon as the indicator is added.
- **Indicators on indicators.** An indicator's Source can be another indicator's line on the chart, as on TradingView: an RSI of an EMA, a moving average of a script's plot, a script acting on another script's signal. Built-ins and scripts in every language can (in Luau, JavaScript and Python, `bar[src]` or `bars[src]` gives the line); the list never offers a loop, links come back with the layout, and removing the source puts its readers back on their own.
- **Text in any language shows.** Characters the app's fonts don't have (Chinese, Japanese, Korean, Arabic, Hebrew, Thai, the Indian scripts and more, in your scripts and names) are drawn with your computer's own fonts, without making the app any bigger.
- **30-Day Fed Funds Futures** (`CBOT:ZQ1!`): search "ZQ" or "fed funds futures". Prices are 100 minus the month's expected average rate, delayed 10 minutes.
- **Your own scripts in the Indicators dialog.** "My scripts" lists the indicators and strategies in your scripts folder, by their titles; search finds them too, and they can be starred.
- **Strategies can recalculate on order fills.** A Pine strategy with `calc_on_order_fills = true` runs again right after each fill, as on TradingView, so it can place orders for the rest of the same bar (a take-profit right after an entry, say). Thanks to the author of pull request #8.
- **An exchange's symbols at a glance.** Type an exchange and a colon in the symbol search (`HYPERLIQUID:`, `BINANCE:`, `OKX:`) to list its symbols, the biggest coins first.
- **Fib levels you choose.** Select a Fib retracement or trend-based extension and press Levels on its bar: switch each level on or off, change its value or colour, remove it, or type a new one (1.272, -0.27). Reset levels brings back the standard set, and the star keeps your levels for new Fibs. The retracement's levels above 1 (1.618, 2.618…) are there, off to start with.
- **Hyperliquid perpetuals.** Search for a coin and pick its Hyperliquid perpetual (`HYPERLIQUID:BTCUSDC.P`, named in USDC as on Hyperliquid): chart, live bars, watchlist quotes, the hourly funding rate in the legend and the DOM's order book. The markets of Hyperliquid's builder-deployed dexes are there too (gold, silver, oil, forex, stock indices and stocks): search for `GOLD`, `SP500` or `EURUSD`, a company (Tesla) or a name like SpaceX; they read `XYZ_GOLDUSDC.P`, the dex first, and show gold's picture, the currency's flags, the index's badge or the company's logo. Hyperliquid keeps only its newest 5000 candles per interval, so 1-minute history goes back about 3.5 days (daily goes back to the listing).
- **Alert or line from the price scale.** A + follows the pointer beside the price scale: click it to add an alert at that price (the alert dialog opens with it filled in) or draw a horizontal line there; the dashed line stays on that price while you choose.
- **Drawing templates.** The Template button on a selected drawing's bar saves its style under a name (two setups for the same Fib, say) and applies a saved one with a click. Reset to default puts the drawing, and new ones of that kind, back to the standard style.

### Changed
- **One Opentern at a time.** Opening Opentern while it's already open brings the open window forward instead of starting a second copy, so settings and layouts are never saved over each other.
- **The Windows installer looks like the others:** `install.ps1` shows the green logo, a progress bar and ticks in Windows Terminal and PowerShell's console, and plain lines anywhere else.
- **The agent writes Pine.** Asked for an indicator or a strategy, Claude or Codex writes it in Pine Script unless you ask for another language, and keeps a script's own language when changing it.
- **Drawing tools simply stay picked.** Draw as many as you like with a tool until you pick another, the cursor, or press Esc. The "Stay in drawing mode" button is gone.
- **Replay controls** match the favorite drawings toolbar: the same rounded bar, icon buttons with their names in tooltips, and dividers.
- **Indicator settings:** a long input name wraps instead of running under its box. Clicking a number puts the cursor where you click instead of selecting it all, and ↑/↓ step it by the script's `step` (1 for whole numbers when a script sets none).
- **Easier to hit:** indicator names and their buttons on the chart are a little larger.
- **Date ranges (1D to All) no longer jump:** the chart shows the chosen range from the first bars on, instead of showing them at the old zoom for a moment and then snapping.
- **Top bar icons** are the size of the drawing toolbar's, and undo and redo look like its buttons.
- **A tidier Script Editor explorer:** rounded rows like the watchlist's, headings in plain case, folders whose arrows turn as they open, a rename box that lines up with the name, and a script you drag follows the pointer to the folder it lands in.
- **Calmer panels:** the bottom panel's tab strip has more room above and below, the line that resizes it is bolder, and the open file in the Script Editor is marked along its bottom edge.
- **A deeper equity chart in the Strategy Report.** Its legend turns Equity, Buy & hold and Drawdown on and off; it shows money or percent, on a linear or log scale, with dates along the bottom. Drag across it to zoom into a range and see that range's profit, drawdown and trades; double-click to see everything again. Click it to show that day on the price chart. The worst drawdown stands out in the drawdown strip, the tooltip lists the trades that closed on a bar, and the three lines' colours are in Strategy settings. Buy & hold now matches the report's Buy & hold return. Thanks to the author of pull request #9.

### Fixed
- The countdown to the bar's close on the price label ticks once a second. It used to pause for nearly two seconds and then catch up.
- The macOS installer stopped with "left_part?: unbound variable" before downloading. macOS runs the script with an older shell that misread the progress bar's dots.

## 0.3.7 (2026-11-26)

A favorite drawings toolbar, keyboard shortcuts you can change, and drawing tools that stay picked.

### New
- **Favorite drawings toolbar.** Star a tool in the drawing toolbar's lists and it gets a one-click button on a small bar that floats over the chart. Turn the bar on with the star in the drawing toolbar. Drag it anywhere over the chart by its handle, stand it upright, fold it down to its handle, or drag its tools into another order. It shows only over the charts and stays out of snapshots. Thanks to the author of pull request #4.
- **Your own keyboard shortcuts.** In Settings → Keyboard shortcuts, press the pencil beside a shortcut and then the new keys; Delete takes one off, and a changed one has a button back to its default. Keys two shortcuts share are marked with a "!" (the one you set wins). Menus and tooltips show yours.

### Changed
- **A drawing tool stays picked** after you place a drawing, so you can draw another right away; Esc puts it down. Turn off "Stay in drawing mode" in the drawing toolbar to go back to the cursor after each one.
- **The drawing toolbar's buttons lose their little arrows.** Click a tool that's already picked, or right-click any of them, to open its list.
- **Stars are smooth and crisp** wherever you mark a favorite.
- **The bar for a selected drawing** matches the new floating bars, and opens clear of the favorite drawings toolbar.

### Fixed
- **The bar for a selected drawing no longer shows over Settings** (or the market overview, a maximized panel or a snapshot).
- **Long tool names no longer run into their shortcut** in the drawing toolbar's lists; a menu shortens a name rather than overlap.

## 0.3.6 (2026-11-26)

Updates and installs that finish, even on a patchy connection or a full disk.

### Fixed
- **Updates no longer freeze at 100%.** Opentern waited without limit on a GitHub download server that didn't answer, and could sit there for minutes before checking the download; it now gives up on such a server in seconds and uses another. Downloads come in pieces, so one that stalls or drops carries on from where it stopped.
- **A failed update says why and offers Try again,** in plain words (no connection, no room, a damaged download), instead of leaving the bar where it was. Each step shows as it happens: downloading, checking, installing.
- **Half-finished updates no longer pile up** next to the app; what an interrupted update left behind is cleared on the next launch.
- **The install command no longer fails when the temporary folder is full:** it downloads to your cache folder instead, tries again when a connection drops or stalls, and says what went wrong in words when it can't.
- **The Windows install command** tries again on a dropped connection, and always checks the download against the release's checksums.

## 0.3.5 (2026-11-26)

Nine languages, a new agent panel with an effort slider, smoother motion throughout, and a full check of the script engines.

### New
- **Opentern in your language.** German, Spanish, French, Portuguese (Brazil), Swedish, Russian, Hindi, Japanese and Simplified Chinese, besides English. It follows your system's language; pick another in Settings → General → Language. Your scripts, saved names and tickers stay as they are.
- **Effort.** Choose how hard Claude or Codex thinks, with a slider under the models in the agent panel. Each chat keeps its own; until you pick one, your agent's own setting applies.

### Changed
- **A new agent panel.** Soft rounded cards in your agent's own colours, usage limits behind a button as meters that show how much of their time has passed, steps in one tidy pill, coloured code that scrolls sideways, a copy button under replies, clear permission buttons and how long a turn takes. Click the logo to choose an agent or account, and a chat's title to rename it. The message box stays at the bottom, and an arrow takes you back to the end when you've scrolled up.
- **The agent icon** gets a dot when an agent needs you or is done.
- **Smoother motion.** Menus, dialogs and tooltips ease in and out, and hovers ease instead of snapping.
- **Fewer tooltips.** Buttons that say what they do no longer repeat it.
- **"Theme" beside a colour** in chart settings is a plain underlined link.
- **Signing in to Claude Code or Codex** happens in your terminal: the agent panel shows the command to copy, for the account you picked. Opentern takes no part in it.
- **Claude Code knows it's running in Opentern**: the app tells it its name, as it does Codex.

### Fixed
- **Claude Code signed in through environment variables** (a long-lived token, Amazon Bedrock, Google Cloud, Microsoft Foundry) is no longer ignored in the agent panel.
- **Codex's models.** The menu kept the list from the first time it was opened; now it loads the current one each time you start Opentern, like Claude's.
- **Luau, JavaScript, TypeScript and Python scripts.** Averages and highs/lows can change their length from bar to bar, a quantity of 0 orders nothing, levels follow their inputs, and daily bars carry their session's opening time. In Python, an infinite value no longer stops the script, orders take a series for their size and targets, and other programs' output can't break it.
- **Pine scripts.** A full check of the engine fixed many scripts that stopped with an error or showed wrong values: `request.security` with your own functions, variables, loops and several symbols; dates, weeks and sessions on US daily charts; hourly bars on NSE and other exchanges that don't open on the hour; moving averages and highs/lows whose length changes; inputs grouped with named constants; date inputs; deleted drawings; risk rules in strategies; and over 80 missing functions. Strategies now follow the default FIFO closing rule, fill limits without slippage and wait for stop-limits. Heikin Ashi requests, `varip`, `indicator(timeframe = …)` and version 3 and 4 scripts now work. Thanks to the author of pull request #3 for pointing at the first ones.
- **Panels in narrow windows.** Every bottom panel fits half a laptop screen: tabs scroll sideways with the open one in view, Paper Trading and the Strategy Lab stack, and the Strategy Report, screener, seasonality table, Script Editor and alerts log no longer run past their edges or overlap.
- **Dividend, split and earnings details** no longer show through a dialog.
- **The opacity slider's handle** covers the end of its line at 100%.

## 0.3.4 (2026-11-24)

A sharper, calmer look throughout, symbol search with prices, a custom colour picker, a keyboard-friendly script explorer and three new indicators.

### New
- **Three more built-in indicators** (102 in all): Moving Average Adaptive (Kaufman's KAMA: quick in a trend, flat in chop), Moving Average Hamming, and Moving Average Channel (averages of the highs and the lows, the band between shaded).
- **Pick any colour.** The colour picker has a Custom tab: a saturation and brightness square, a hue bar, and hex and RGB fields, with the opacity slider under both tabs.
- **Prices in symbol search.** Each result shows its price and today's change, so you can tell listings apart before opening one.
- **Star a symbol from search.** The star on a row (or Shift+Enter) adds it to the open watchlist, or takes it out, without leaving the search.
- **Search by coin name and with typos.** "bitcoin" finds BTCUSDT, "solana" finds SOL; "RELAINCE" still finds Reliance; a word of a company's name works too ("Mahindra").
- **Every exchange in the Calendar.** Besides the US and India, the Calendar lists holidays, half days and special sessions for London, Frankfurt, Paris, Zurich, Tokyo, Hong Kong, Shanghai, Seoul, Taipei, Singapore, Sydney, Toronto and São Paulo, grouped by region (folded until you open one).
- **The script explorer works from the keyboard.** Click a file, then use the arrows (Right opens a folder, Left closes it), Enter to open, F2 to rename, Delete to delete (it asks first), Ctrl+C or Ctrl+X then Ctrl+V to copy or move a script into a folder, and a letter to jump to a name.

### Changed
- **Simpler alert setting.** Settings → General → Alerts asks one thing, what happens when Opentern is closed: pause alerts, keep checking them, or keep checking even after a restart.
- **A cleaner, sharper look.** Deeper night-sea surfaces in clear layers, hairline borders, a fresher tern green, and a rich deep-green fill for whatever is selected or switched on; the light theme is paper white on cool grey. Dialogs get a proper header, a firmer border and a soft green glow from the top; radio buttons, sliders, chips and text selection are redrawn to match.
- **See-through colours look like themselves.** A colour swatch shows the colour as it will look at its opacity (hover for the percentage), instead of a grey checkerboard.
- **The alert dialog, redesigned.** Labels sit above their fields, related settings share a row, the message box lines up with the rest, and where to notify you is a row of toggles (Notification, Sound, Webhook, Telegram, Email) that reveal their settings when on.
- **Watchlists switch by name.** The row of letter circles is now a strip of list names that scrolls sideways, with the open list highlighted.
- **Brighter heatmap tiles in light mode.** Small moves are clean greens and reds instead of dusty teal and pink.
- **Renaming in the script explorer** happens in a slim box in the row itself.
- **Data providers** has a clearer icon in Settings.
- **Search puts your symbols first.** Symbols in your watchlists or opened lately come before others that match as well, and the part that matched is highlighted.
- **Tab switches the search's kind tabs** (Shift+Tab goes back).

### Fixed
- **Lit buttons side by side no longer touch** (log and auto under the chart).
- **Enter opens the highlighted symbol at once.** On some networks it waited up to several seconds for a coin lookup, so it took a few presses (or a click).

## 0.3.3 (2026-11-23)

A smoother agent panel, 6-month and 1-year returns, a font of your choice and friendlier symbol search.

### New
- **The agent can move around the chart like you do.** It can pan back and forth, zoom in and out, fix the price scale to a range and ask what's in view, so it looks at the right bars before judging trades.
- **6-month and 1-year returns.** The market overview gains a 6M period (beside 1D, 1W, 1M, 3M, YTD and 1Y), custom indices show 6M and 1Y returns for the index and every member (and can colour their map by them), and the screener has Perf 6M and Perf 1Y columns you can sort and filter by.
- **Choose the app's font.** Settings → General → Font offers Inter, Geist, Onest, Plus Jakarta Sans, DM Sans, Figtree, Outfit, IBM Plex Sans and JetBrains Mono, each shown in its own face; the change applies at once.
- **Select and copy text in the agent panel.** Drag across part of a reply or one of your own messages to select it, then copy it.

### Changed
- **Friendlier symbol search.** Words can come in any order and match the ticker, the name or the exchange, so "NSE VIX" finds India VIX and "apple nasdaq" finds Apple; spaces and punctuation don't matter ("SP500" finds the S&P 500), and "NSE: VIX" works with a space after the colon.
- **Queue a message while the agent works.** With text in the box, the round button now queues it (as Enter already did) instead of stopping the agent; with the box empty it still stops.

## 0.3.2 (2026-11-22)

Logos and commodity icons show up again, and currency pairs show their flags.

### New
- **Currency pairs show their two flags.** USD/INR, EUR/USD and other forex pairs show both countries' flags overlapping, like TradingView, in place of a letter circle.

### Fixed
- **Logos show up again.** Built-in logos (coins like XRP and NEAR, Indian companies like SBI, TCS and Infosys) and commodity icons like silver's showed letter circles for minutes on the market overview, in search and in watchlists. They waited in line behind logos still being looked up online; now they appear at once, and logos that do need downloading come four at a time instead of one by one.
- **Very long custom formulas and Pine expressions no longer overflow on Windows.** A formula of thousands of symbols, or a Pine expression with thousands of terms, gives a plain error or works instead of closing the app.

## 0.3.1 (2026-11-22)

Markets around the world: indices, Treasury yields, VIX and the dollar index with every exchange's hours and holidays, crypto perpetual futures with their funding rates, commodity icons and spot prices, and more logos. Coins an exchange delists carry on where they still trade, and Opentern is sturdier against damaged or hostile data.

### New
- **Indices from around the world.** The Nikkei 225, FTSE 100, DAX, CAC 40, Euro Stoxx 50, SMI, Hang Seng, SSE Composite, KOSPI, Taiwan Weighted, Straits Times, ASX 200, TSX and Ibovespa, plus the S&P 500, Nasdaq 100 and Composite, Dow and Russell 2000, each with its exchange's trading hours. They show their country's flag, or TradingView's badge.
- **Holidays, half days and lunch breaks for every exchange.** Tokyo, London, Frankfurt, Paris, Zurich, Hong Kong, Shanghai, Seoul, Taipei, Singapore, Sydney, Toronto and São Paulo know when they're closed, so an index's open/closed dot, its daily bars and the Calendar widget are right. The Calendar lists the chart's exchange first, and each of its lists folds away with a click on its title (remembered).
- **Rates, volatility and the dollar.** US 3-month, 5-, 10- and 30-year Treasury yields, VIX, the Nasdaq 100's VXN, India VIX, the MOVE bond volatility index and the U.S. Dollar Index.
- **Crypto perpetual futures.** Binance, Bybit and OKX perpetuals (`BTCUSDT.P`) chart and stream live like spot, show up in search tagged "perpetual", and the legend shows the funding rate and the time to the next payment. The depth-of-market ladder works for Binance and Bybit perpetuals.
- **Commodity icons.** Gold and silver show stacked bars, platinum, palladium and copper ingots with their element, oil a barrel, natural gas a flame, and the farm and livestock markets their own pictures, in place of letter badges.
- **Logos for many more coins and stocks.** US and large Indian stocks get logoapi.sh's logos, coins quoted per thousand (1000SATS) show the coin's logo, and 27 more Indian companies (BHEL, GAIL, ITC, L&T, NTPC, ONGC, Sun Pharma and others) come built in. Fewer letter circles in the watchlist, search and market overview.
- **Spot commodities from OANDA.** With an OANDA key, search lists spot gold, silver, platinum, palladium, copper, WTI, Brent, natural gas, wheat, corn, soybeans and sugar beside the futures. Search tags each row "futures" or "spot".

### Changed
- **Clearer market names.** "US Markets" and "Indian Markets" replace "US stocks" and "India" in the market overview, hotlists, the Calendar, onboarding and settings; the screener's preset is "Indian large caps".
- **The crypto map shows the 150 most traded coins, sized by 24-hour volume on Binance.** Opentern no longer uses CoinGecko: its free API is meant for testing, and every price already comes from the exchanges.
- **Delayed futures prices are labelled.** Commodity futures charts show "Delayed 10 min": free sources get CME's and ICE's prices at least 10 minutes late. Daily closes are the official settlements.
- **A friendlier installer on Linux and macOS.** `install.sh` shows the Opentern logo, the version it's installing and a progress bar with megabytes, then a tick for each step. Piped into a log, it prints plain lines as before.

### Fixed
- **The first launch no longer stumbles over the price cache.** Opening it from several places at once could fail with "database is locked" (or, on Windows, "duplicate column name") on a new install.
- **A damaged or hostile script, formula or reply can't crash Opentern.** Testing with millions of mangled inputs found ways to: a Pine script nested thousands deep or asking an indicator for an absurd length, a custom formula thousands of brackets deep, or a price feed with impossible dates. Each now gives a plain error instead.
- **Safer with data from the internet.** The Interactive Brokers gateway's certificate check is only skipped for a gateway on this computer (an address that merely started with "localhost" used to pass). Requests that carry your API keys never follow a redirect to another site. Downloaded logos can't read files from your disk or ask for huge amounts of memory, live data connections cap their message size, and exported trade lists can't carry spreadsheet formulas from a script.
- **Coins an exchange stops trading carry on elsewhere.** When an exchange drops a pair (Binance still lists Monero's XMRUSDT, frozen since February 2024), watchlists, charts and price alerts move to the same coin where it trades, with a note saying so. The market overview no longer compares today's price with a delisted pair's old closes (it once showed Monero +331% on a new workspace).

## 0.3.0 (2026-11-21)

Paper accounts work like real ones, symbol search keeps up with your typing, data holds steady when Yahoo struggles, and Seasonality is right for US stocks and custom indices.

### New
- **A fair warning about Python, once per computer.** The moment a Python script shows up anywhere in your scripts folder (written by you, an agent or copied in), a short note says that Python scripts run with the same access as any program on your computer, and that Pine, Luau and JavaScript are sealed inside Opentern.
- **Set up paper accounts.** New account and the account menu's Account settings open a dialog for the name, starting capital, currency, commission, slippage and leverage. Changing the starting capital later keeps the profit and loss made so far.
- **Paper trading costs and limits, like a real account.** Slippage moves market, stop and trailing fills against you (limits fill at their price). Orders need buying power: equity times leverage, 1× being cash only, so an order larger than the account is refused. With leverage, falling below half the margin in use closes every position (a margin call, with a notification). Hover the account summary for margin in use and buying power.
- **Paper accounts in your own currency.** Keep an account in USD, INR, EUR and others: profit and loss on symbols priced in other currencies is converted at live exchange rates.
- **The agent manages paper accounts.** It can create accounts with these settings, change them, switch, reset and delete.
- **Search custom indices.** A search button beside Compare filters the list by name.
- **Search watchlists.** A search button in the Watchlist header finds a list by name; Enter or a click switches to it.

### Changed
- **Symbol search shows your recent picks.** With nothing typed, each tab lists the symbols you picked lately (the starter list until there are some).
- **Paper accounts are cash accounts unless you give them leverage.** Existing accounts start at 1×, so orders bigger than their equity are now refused. Reset account keeps all of an account's settings.
- **The paperclip opens the file picker straight away.** Paste image is gone from it; Ctrl+V still pastes an image.

### Fixed
- **"Check your internet connection" only when it's the connection.** When something else answered lately (a crypto stream, another request), a failure names the service instead ("Yahoo isn't answering right now (error 502).", "Couldn't reach Yahoo."), with a warning icon rather than the no-signal one. Messages clear as soon as data arrives.
- **Yahoo falls back to its second server.** When query1.finance.yahoo.com is slow (5 s) or failing, query2 is asked, and whichever answered goes first next time. Indian charts were getting 502s after 15 s on the first while the second answered at once.
- **Custom index charts survive a failing member.** One member's failed price update no longer stops the whole index: it's retried on its own and counts at its last price meanwhile. Opening an index reads its members' history from the cache (fetched in parallel) instead of downloading all of it again.
- **Members Yahoo no longer has are flagged.** A renamed or delisted member (Yahoo's 404) gets a warning sign by its name and in the index's header, saying to replace it.
- **Symbol search answers as you type.** S&P 500 and NIFTY 500 stocks and the exchanges' coins show at once; Yahoo's results join when they arrive, and a slow Yahoo no longer holds the list back.
- **Watchlist changes line up under Chg%,** with more room after the price.
- **The price cache opens reliably on a first launch.** Several parts of the app open it at once, and its one-time setup could fail with "database is locked".
- **Seasonality puts US months in the right column.** Months of symbols whose exchange is behind UTC (US stocks and funds) showed a month early, and each year's January return went under the year before. The Seasonality indicator on daily charts had the same one-day slip.
- **Custom indices' weekly and monthly bars are right when weights change mid-month.** Seasonality and 1W/1M charts of a custom index now build each week and month from the index's own days, so a member joining or a rebalance no longer throws off that month and the next.
- **Settings shows over a maximized bottom panel.** It used to open behind it; the panel comes back as it was when Settings closes.
- **The map of every index closes like the market overview.** Clicking its button again goes back to the index or Compare you were on.
- **Claude's model menu lists the newest models.** It now asks Claude Code which models are current (Fable 5.2 instead of 5.1), once a run when the menu opens.
- **The side panel lines up with a maximized bottom panel.** Their cards now start at the same height.


## 0.2.15 (2026-11-20)

Comparing many custom indices no longer freezes the app, and index boards show the watchlist's prices.

### Fixed
- **Compare indices no longer freezes the app.** With many custom indices, every new price used to redo every index's whole history (about a second each time, and again every 30 seconds while the market traded). Now only today is redone for the indices whose stocks moved, in a few milliseconds.

### Changed
- **Custom index boards match the watchlist.** Each member's price and day change are the watchlist's.
- **P/E and EV/EBITDA follow the live price** on index boards and in Compare.
- **Members' price history and company figures download once a day** instead of every 15 minutes; the live price keeps today's numbers moving in between.

## 0.2.14 (2026-11-20)

A symbol now shows one price everywhere at once, and pairs on Bybit, OKX, Coinbase and Kraken update live in the watchlist.

### Fixed
- **The watchlist and the market overview show the same price.** The watchlist takes the overview's prices as they arrive, instead of fetching on its own 30-second cycle and differing by a few paise.
- **A symbol shows one price everywhere at once.** A new price from the watchlist, a chart, the market overview or a hotlist now moves that symbol's price in all of them together.
- **Charted symbols are kept in step in every market**, not just US stocks: an Indian stock on a chart and not in the watchlist now moves its map tile and hotlist row too.

### Changed
- **Bybit, OKX, Coinbase and Kraken pairs update live in the watchlist** (about once a second), instead of every 30 seconds.
- **The India hotlist shares the watchlist's request** to Yahoo and works out its change the same way.

## 0.2.13 (2026-11-20)

Custom indices now update live, and Yahoo prices come from one shared batch request.

### Changed
- **Custom index stocks update live.** Each stock's price and day change in the Indices tab, and the index's own figures, now follow the market every 30 seconds while it trades (they used to wait up to 15 minutes).
- **Far fewer requests to Yahoo.** The watchlist, custom indices and the market overview now share one batch request per 20 symbols every 30 seconds (it used to be one request per symbol). Symbols whose market is closed are checked once after the close, then every 5 minutes. The market overview's full refresh runs only while its market trades, plus once after the close.

### Fixed
- **Email alerts are safer.** The mail library is updated to fix six security issues (published 17 Nov 2026), including a hostile mail server forging replies or crashing the app.

## 0.2.12 (2026-11-15)

A heatmap of all your custom indices, pre-market and after-hours prices
for US stocks, and warnings when prices stop updating.

### New
- **A map of all your custom indices.** The new map button beside + in the Indices tab shows every index's stocks on one heatmap, like the market overview: each index is a group sized by its stocks' market caps, each stock sized by its weight in the index, coloured by the change over 1D, 1W, 1M, 3M, YTD or 1Y. An index's name shows its own change for the period; click it to zoom in (Esc zooms out), and click a stock to chart it.
- **Pre-market and after-hours prices for US stocks, as on TradingView.** While they trade, the chart's legend shows a sun (pre-market) or moon (after hours) with the price and change on hover, and the price scale gets its own label and dotted line (Chart settings → Scales and lines). The market overview says when extended hours are on, and each tile's card shows the extended-hours price. Candles and the day's change stay on the regular session.

- **Says when prices stop updating.** If a live chart's prices haven't moved for 5 minutes while its market is open, the legend says "No new prices for 7 min". The watchlist dims such a price (hover says how long), and the market overview's header shows a warning.
- **Says when Yahoo is missing prices.** A BSE stock whose daily history Yahoo lacks (most of them) says so and points to its NSE listing. An Indian chart missing a recent special session (Muhurat trading) says that session isn't on it.
- **A daily backup of your workspace.** Opentern saves your watchlists, layouts, drawings, alerts, templates and paper accounts to `backups/` once a day and keeps the last week. Restore one from Settings → Export and import → From a file.
- **The price cache's size, and a button to clear it** (Settings → General → Price cache). It asks first and says what may not come back: sources keep only recent minute and hour bars, and special sessions recorded live are never downloadable.
- **Untested data providers are marked Beta.** Alpaca, OANDA, Upstox and Interactive Brokers haven't yet been tried with a real account.

### Changed
- **The market overview keeps up while markets trade.** Prices refresh every 30 seconds while the market is open (crypto always), instead of every 3 minutes.

### Fixed
- **Logos no longer cover the names on the Indices map.** A short tile put its logo on top of the ticker (the side panel open leaves less room); the text now moves down out of its way, or the logo is left out.
- **The Indices tab fits a narrow window.** With the window tiled narrow and the side panel open, the board's header pushed the board past the panel's edge and the index cards' changes ran out of their cards; the header now takes two rows and the cards drop the value before the change.
- **US stocks' 1D change in the market overview is right before the open and after the close.** It compared the extended-hours price with the close two sessions back (NVDA showed −6.18% before the open on 12 Nov when it had closed +2.87%). It now shows the regular session, like the chart and watchlist.
- **No more flat extra bar at the close on intraday stock charts.** Opened outside market hours, a 15m or 1h chart ended with a flat, volume-less bar at the closing price (Yahoo's latest-price point). The last bar now ends at the official close instead, and those bars are removed from the cache once.
- **Weekly and monthly stock charts include today.** The current week (month) ended at yesterday, and today showed as a separate bar dated today (NVDA's weekly chart had a "week" starting on Thursday). Weeks and months are now built from daily bars, as Indian stocks' already were.
- **A live chart no longer freezes after the computer sleeps or changes network.** A connection that died without closing kept the chart showing "live" on old prices for good. Quiet connections are now pinged, and one that doesn't answer is reconnected (within about a minute).
- **Watchlist stock prices keep updating while Binance is unreachable.** They used to stop until Binance answered again.
- **Live hourly bars from Upstox, IBKR and Alpaca line up with their history.** They were placed on the clock hour (09:00) while history starts at the session open (09:30, or 09:15 for NSE).
- **A damaged price cache is set aside and downloaded again** instead of being half-read.
- **The diagnostics log stays under about 2 MB.**
- **Charts build bars for sessions Yahoo skips.** During trading Yahoo's bars leave out (Muhurat trading), its latest-price point could keep the chart from building bars from the live quote.

## 0.2.11 (2026-11-10)

Diwali's Muhurat trading and special Saturday sessions now count in Indian
prices and update live while they run.

### Fixed
- **The day after a special session shows the right change.** Diwali's Muhurat trading (Sunday 8 Nov 2026) and special Saturday sessions were left out, so the watchlist and market overview measured from the session before (SBIN +2.31% instead of +2.56% on 9 Nov) and disagreed with the hotlists.
- **Special sessions update live while they run**, like a normal trading day: charts get the session's bars, weekly and monthly bars include it, and the market dot shows it as open. The Calendar lists upcoming ones with their times.

## 0.2.10 (2026-11-10)

Send messages to the agent while it works, paste images with Ctrl+V, and
chart legends whose buttons stay in reach.

### New
- **Send messages while the agent works.** Press Enter mid-task and your message waits under the reply, dimmed (× takes it back); the agent gets everything you queued as soon as it finishes, as in Claude Code. Stop puts them back in the box.

### Changed
- **Ctrl+V pastes an image in the agent panel** (Cmd+V on macOS), instead of Alt+V. Copied text and files paste as before.

### Fixed
- **Shift+Enter starts a new line** in the agent's message box; it used to send.
- **A strategy with many inputs can be hidden or removed again.** Its name on the chart used to run past the edge (with the side panel open), taking its buttons with it; a long name is now cut short with "…" (the full one shows on hover) so the buttons always fit.
- **SQLite security update.** The built-in database engine is updated for a medium-severity SQLite advisory (RUSTSEC-2026-0459).

## 0.2.9 (2026-11-09)

Custom indices always reach back to their oldest stock, and scrollbars that
stay out of the way.

### Changed
- **Scrollbars stay out of the way.** They no longer appear just because the pointer is over a list or table (on the Indices board they covered the tickers); they show while you scroll and fade out a moment later. Pointing right at the edge still shows a bar to drag.
- **Custom indices always reach back to their oldest stock.** The "Index starts" column is gone: each stock joins the day after it lists, the way NSE adds stocks, with the level carried on unbroken. Indices with a stock set to "With it" are rebuilt this way once.
## 0.2.8 (2026-11-08)

Clearer wording for when a custom index starts.

### Changed
- **Clearer wording for when an index starts.** On an index's board, the "Counts from" column is now "Index starts": **Without it** (the default) starts the index with the older stocks and adds a newer one the day after it lists, keeping their full history; **With it** makes the index wait until that stock has prices, so it's in every day of the index.

## 0.2.7 (2026-11-08)

A heatmap of the whole market, custom indices that reach back as far as
their oldest stocks, and splits that no longer read as crashes.

### New
- **Market overview**: a button beside the layout grid button (or `Alt M`) swaps the charts for a heatmap of the S&P 500, the NIFTY 500 or the 150 largest coins. Tiles are sized by market cap, grouped by sector and coloured by the change over 1D, 1W, 1M, 3M, YTD or 1Y. Hover a tile for its price, change and market cap, click a sector to zoom in, click a tile to open its chart, and press Esc to go back. Watchlists and the agent stay on the right.
- **The agent can use the market overview**: ask it to show a market, or which sectors and stocks led over a period.
- **Custom indices reach back as far as their oldest stocks.** A stock now joins an index the day after it lists, the way NSE adds stocks, with the level carried on unbroken, so one young stock no longer cuts ten years of history down to one. Indices you've made are rebuilt this way once.
- **Choose when each stock counts.** On an index's board, "Counts from" switches a stock between its listing and the index's start (the index then begins only once that stock has prices), and a tick box leaves a stock out altogether. The agent can do both.
- **See how much history each stock has.** A History column shows how far back each stock's prices go, and the index row shows the index's own.
- **Lakhs and crores, or millions and billions, per market**: Settings → General → Large numbers sets how market caps and volumes read for US stocks, Indian stocks and crypto. Indian stocks start in crores.

### Fixed
- **Splits and bonus issues no longer read as a crash.** Charts, custom indices, the screener and Seasonality kept prices saved from before a split, so a 2-for-1 split showed as a 50% fall in 1M and 3M returns. Saved prices are now checked against the source each time and downloaded again when the source has adjusted them. Prices a source hasn't adjusted yet are adjusted on the split's date, including Yahoo's hourly bars when a split and a bonus land on one day (an index holding Bajaj Finance broke in two on hourly charts). The S marker under the chart reads "Split or bonus" for Indian stocks. Saved prices are downloaded once more after updating.
- **Custom index P/E, EV/EBITDA, margins and growth are right for companies that report in another currency.** Yahoo gives Infosys's earnings in dollars and its market cap in rupees; they're now in one currency, so an index holding it no longer shows a negative or far-off P/E.
- **An index's P/E and other ratios count what the index holds.** Each company counts for the slice the index owns (its free float, or its share in an equal-weight index), as NSE works out its indices' P/E, and a one-stock index reads its stock's own P/E.
- **No index day lost or counted twice.** A stock that doesn't trade on a day (a suspension, or a gap in the data) counts at its last price instead of dropping the day, and new weights take over exactly where the old ones left off.
- **1M and 3M returns on the Indices board are calendar months,** as in the market overview.
- **Sector changes in the market overview add up.** A sector's change was its stocks' changes weighted by today's market caps, which overstated winners over longer periods; it's now how much the sector's total market cap moved.
- **New coins show up in search the day they list.** Exchange coin lists now refresh daily instead of weekly, and a search that finds no coin asks Binance about it right away.

## 0.2.6 (2026-11-05)

Move your Opentern to another computer, as a file or straight over your network.

### New
- **Export and import** in Settings: tick what to bring (watchlists, layouts, drawings, templates, favorites, alerts, custom indices, paper accounts, scripts, screener and Strategy Lab settings), then save one file you can email or copy across.
- **Send to another device**: a six-digit code connects two computers on the same network, with no account and no server, and everything sent is encrypted.
- **Merge or replace when importing**: pick what to bring in, keep what's already there or replace it. A backup is saved first, and importing that backup puts everything back. API keys and passwords never leave the device.
- **Windows and macOS permissions have a way back.** Windows Firewall and macOS's Local Network prompt only ask once. If one was turned down, the send page has a link to allow it (Windows asks for administrator permission).

### Security
- **Imported files are treated as untrusted, however they arrive.** Imported Python scripts never start by themselves, not even from a saved layout: press Run in the Script Editor and confirm first. Saved layouts, alerts and screens can't reach scripts outside the scripts folder.
- **Importing shows what a file really contains**, read from its contents: how many Python scripts, and where its alerts would send things. It can't change a mail or Telegram setup you already have, so your saved password never goes to another server.
- **Transfers stay on the local network.** Internet addresses are refused, a device pretending to be the sender gets only three tries per search, and a connection has 20 seconds to prove the code.

### Fixed
- **Luau scripts can no longer load files or raw bytecode.** `require` and `loadstring` were reachable and are gone.
- **Only web pages open from links.** A news item or other link that points at a file, a program or another app is now ignored.
- **Odd symbol names can't write outside their folders** when saving logos or snapshots.
- **Script names Windows can't save** no longer stop an import: the rest come in and the message lists which were left out.

## 0.2.5 (2026-11-02)

Alerts keep working with Opentern closed, stock charts show earnings and
extended hours, and the chart's clock picks any timezone.

### New

- **Choose where scripts and snapshots live.** Settings → General has a
  folder for each: change it, open it, or go back to the default. Scripts
  on the chart, the Script Editor and the agents follow the new scripts
  folder.
- **Pick the chart's timezone from the clock.** Click the time at the
  bottom right for Exchange, UTC, Local or any of 80 cities (search by
  name or offset). Chart settings → Symbol has the same list.
- **Alerts keep working when Opentern is closed.** Closing the window
  hands your active alerts to a small background process that watches
  them and sends them as usual (desktop, sound, webhook, Telegram,
  email); opening Opentern takes them back, with a note of any that fired
  meanwhile. Settings → General → Alerts turns it off, or on at login
  too.
- **Alerts on other symbols and indicators off the chart.** An indicator
  or script alert keeps working after you switch symbol or remove the
  indicator, and price alerts work for any symbol, not only ones in the
  watchlist.
- **Earnings on the chart.** Stocks show an E marker for each report:
  green when it beat the estimate, red when it missed, grey for the next
  one. Hover for reported and expected EPS and the surprise.
- **EXT: pre-market and after-hours bars** for US stocks on intraday
  charts, shaded as on TradingView (the EXT button in the bottom bar).

### Changed

- **Settings → General** gathers the folders, the theme and symbol logos
  (Appearance and Symbol logos were categories of their own).

### Fixed

- **Data providers settings fit a small window** instead of running off
  the right edge.

- **Solana's logo is Solana's again:** its purple-to-mint bars on black,
  instead of a teal and green disc.
- **Pasting a copied file into the agent panel works on Windows** (its
  `file:///C:/…` links were read as a computer name and ignored).

## 0.2.4 (2026-10-31)

Seasonality gets a chart of every year side by side, and the agent panel can
now work with Seasonality and custom indices.

### New

- **Seasonality chart.** A toggle at the top left of the Seasonality tab
  switches between the table and a chart with a line per year (the change
  since the year before ended), a dashed average and each year's latest
  change tagged on the right. Hover to see every year's change on that day.
- **An Average row in Seasonality**, above Rises and falls: each month's
  average return over the years shown.
- **The agent works with Seasonality and custom indices.** Claude and Codex
  can read a symbol's monthly returns, and list, read, make, edit,
  rebalance, compare, delete and show custom indices.
- **A refresh button for News.** It turns while headlines load.

### Fixed

- **Seasonality numbers no longer run into each other** when the side panel
  is open or a year's return is in the thousands of percent: columns fit
  their numbers, with fewer decimals when space is short (hover for the
  exact value).
- **News and Hotlists recover by themselves after a dropped connection.**
  They try again 15 seconds after a failure instead of waiting minutes or
  needing the chart reloaded.

## 0.2.3 (2026-10-30)

Build your own indices, see a symbol's seasonality, and choose which Claude
Code or Codex account the agent panel uses.

### New

- **Custom indices.** Make an index from any symbols in the new Indices tab
  (search, or add a whole watchlist). It's weighted by free-float market cap
  as NSE weights its indices, or by full market cap, or equally; starts at
  1000; and rebalances on 1 April and 1 October (or on demand) without the
  level jumping. Chart it like any symbol, with indicators and alerts.
- **An index board.** Each index shows its returns, P/E, EV/EBITDA, revenue
  growth, margins, RSI and distance from its 50- and 200-day averages, then
  every member's, as a colour-coded table or a treemap.
- **Compare indices.** Every index's numbers side by side, as a table or a
  map.
- **Seasonality.** A new bottom tab shows the chart symbol's return for
  every month of every year, with a slider to pick the years and a count of
  rises and falls per month.
- **Agent accounts.** With more than one Claude Code or Codex account on
  the computer, pick which one the agent panel uses from its menu. Each chat
  stays on the account it started with.

### Changed

- **Maximizing the bottom panel gives it the whole window** under the top
  bar, chart included.

### Fixed

- **The agent's message box stays in view** when a long message wraps; past
  eight lines it scrolls.
- **Strategy Lab column titles no longer overlap.**
- **On Windows, links open in the browser** (news links used to open the
  Documents folder).
- **Formulas accept tickers with "&"**, such as NSE:M&M.

## 0.2.2 (2026-10-28)

News covers Indian stocks and follows the coin you're viewing, the Calendar
is easier to read, and the watchlist's daily change for indices is right.

### Changed

- **News draws on many outlets at once.** Every symbol gets a search of the
  past week's news. Indian stocks and indices add the Economic Times and
  Mint. Crypto adds CoinDesk, Cointelegraph, Decrypt and The Block, keeping
  only the headlines that name the coin. Repeats are removed, the newest come
  first, and each headline shows its outlet.
- **A tidier Calendar.** Holidays show as date tiles with the weekday, how
  far off they are and a Closed or early-close tag. Dividends and splits are
  grouped by year, with amounts in the stock's currency (₹25.00, not
  25.0000).

### Fixed

- **The watchlist's daily change for NIFTY, SENSEX and BANKNIFTY matches the
  chart.** It was measured from the close two sessions back, so BANKNIFTY
  could show red on a day it rose.

## 0.2.1 (2026-10-28)

Crypto charts load again on networks that block Binance, and hotlists say
what period their change covers.

### Changed

- **Hotlists show what the change covers:** "Chg% 24h" for crypto (a rolling
  24 hours) and "Chg% today" for stocks (since the previous close).

### Fixed

- **Crypto charts and watchlist prices stay up to date on networks that
  block Binance** (such as Jio in India). Opentern now reads Binance's public
  market data from Binance's own mirror first and falls back to the main
  servers, so charts no longer sit on old cached bars with no prices in the
  watchlist. The order book and crypto hotlists use it too.
- **A blocked data service gives up sooner.** Live connections stop trying
  after 8 seconds instead of hanging for minutes before reconnecting.

## 0.2.0 (2026-10-27)

The agent panel's Claude and Codex can now use the whole app, the way you
would with clicks. Also: Windows shows Opentern's icon and symbol logos, and
a What's new card after each update.

### New

- **The agent can drive the app.** Beyond reading the chart and writing
  scripts, it can change the chart type, zoom to dates, split the window into
  several charts, compare symbols, change an indicator's inputs, draw (trend
  lines, boxes, Fibonacci, long/short positions, labels and more), run bar
  replay, manage watchlists, run the screener, change a strategy's settings,
  run the Strategy Lab (optimize, walk-forward, Monte Carlo, portfolio) and
  apply its results, paper trade, set alerts and open bottom-panel tabs.
- **Learn from real examples.** Ask the agent to teach you a pattern: it
  finds one in the chart's history (loading older bars if it has to), marks
  it, and steps through it in bar replay.
- **The agent can look at the chart.** It takes a picture of what you see to
  check its drawings or read a pattern.
- **What's new after updates.** The first launch of a new version shows what
  changed; Settings → About opens it again.
- **Labels on drawings.** Rectangles and lines can carry a short label.

### Fixed

- **Windows shows Opentern's icon** in Start, search, the taskbar and
  Settings → Apps (the program had none of its own).
- **Symbol logos show on Windows.** The bundled logos were read wrongly on
  Windows builds.

## 0.1.5 (2026-10-27)

A welcome for new installs, Opentern's own green themes, and an agent panel
that asks you to sign in before you type.

### New

- **A welcome on first launch.** Pick the markets you trade (crypto, US
  stocks, Indian stocks, forex, commodities) and each becomes a watchlist of
  its top seven. Then pick your first chart from cards with live prices and
  sparklines, and it opens into the app.
- **Sign in from the agent panel.** If Claude Code or Codex is installed but
  not signed in, the panel says so and signs you in through your browser,
  instead of failing after your first message.

### Changed

- **Tern Dark is the new dark theme**: Opentern's slate with the logo's green
  for accents and selection. It replaces Opentern Dark, and a saved Opentern
  Dark opens as Tern Dark.
- **Opentern Light is greener**: white and pale sage with the logo's deep
  green.
- **The agent panel shows the model it will use** (from Claude Code's or
  Codex's own settings) instead of "Default".
- **The agent's message box is quieter**: no suggested prompt, a softer
  cursor, and "What can I help you with?" until you type.
- **The website** takes the new colours.

### Fixed

- **Watchlist and chart prices agree.** Whichever price arrived last shows in
  both.
- **Dragging the bottom panel up no longer crushes the chart**; it stops
  where the chart still has room, and shrinks with the window.
- **Codex's model list loads on a new workspace** (it never did before a
  script was saved).
- **The website's logo is centred on phones.**

## 0.1.4 (2026-10-22)

Alt W adds the chart's symbol to your watchlist, updates show a progress bar,
and the agent panel finds Claude Code and Codex on Windows however they were
installed.

### New

- **Alt W adds the chart's symbol to the watchlist** that's open, at the end
  of its top section. Drag it from there to wherever you want it.

### Changed

- **Updates show a progress bar** with the percentage in Settings → About.
- **Deleting a watchlist takes one click.**
- **"Install Claude Code" and "Install Codex" link to their install pages**
  when the agent panel can't find them.

### Fixed

- **The agent panel works with npm installs on Windows.** Opentern runs the
  Claude Code or Codex program inside the npm package, and a native install
  wins wherever it is on the PATH.
- **The watchlist menu stays inside the watchlist panel**, and the indicator
  settings' Defaults menu no longer stretches across the window.

## 0.1.3 (2026-10-22)

Every drawing tool gets a keyboard shortcut, watchlists work like
TradingView's, and Hotlists cover Indian stocks.

### New

- **A shortcut for every drawing tool.** TradingView's (Alt T, H, V, C, F and
  Alt Shift R) plus Alt Shift and a letter for the rest, and Alt L, Alt P and
  Alt U for log, percent and auto scale. Settings → Keyboard shortcuts lists
  them all; the keyboard button beside the theme switch opens it.
- **Watchlists like TradingView's.** Sections that fold, rows you drag to
  reorder, a circle per list to switch, and naming a new list right away.
- **Indian stocks in Hotlists.** The NIFTY 50's gainers and losers.

### Changed

- **The update button shows its progress** as a ring that fills while it
  downloads, then a restart icon.
- **Dropdowns fit their choices** instead of cutting them short.
- **About** says what Opentern is.
- **A market analysis terminal.** Opentern now describes itself that way
  everywhere: it's for analysis and paper trading, and never places real orders.
- **One install line for Linux and macOS.** `install.sh` now installs the Mac app
  too.

### Fixed

- **Demo mode shows symbol logos** (the ones bundled in the app; it still never
  goes online).

## 0.1.2 (2026-10-21)

Opentern now runs on macOS and Windows, and the agent panel works with Codex
as well as Claude.

### New

- **macOS and Windows builds.** A universal app for Apple Silicon and Intel
  Macs, and an installer for x64 and ARM64 PCs. Linux gets ARM64 builds too.
- **Pick your agent.** The panel beside the chart works with Claude Code or
  OpenAI's Codex CLI, whichever you have installed. Switch between them from
  the top of the panel at any time; each keeps its own chats.
- **Starred indicator templates.** Star a template and it sits in the top bar,
  one click away.
- **License notices.** Every download includes the licenses of what Opentern
  is built from, and `opentern --licenses` prints them.

### Changed

- **A calmer interface.** Fewer captions and explanations, softer text boxes,
  focus rings and menus, and the drawing toolbar slides in and out.
- **Settings is a gear** at the right end of the top bar.

### Fixed

- **Symbol search scrolls** with the mouse wheel again.
- **Alt+V** in the agent panel pastes an image without also typing a "v".
- **Safer updates.** The libraries that unpack downloads and check secure
  connections are updated to fix known security issues.

## 0.1.1 (2026-10-21)

Strategy results now match TradingView, checked against a TradingView Strategy
Report on BINANCE:BTCUSDT 1D (2017 to 2026, 40 trades): net profit, max
drawdown, profit factor and every trade size are the same, to the cent.

### Fixed

- **Strategies act on closed bars only,** as on TradingView.
- **"% of equity" orders** leave room for the commission and round to the
  exchange's lot size.
- **Pine v6 margin:** entries the account can't pay for are skipped, and
  losing positions get margin calls.
- **Max drawdown** is measured from your highest closed-trade equity.

## 0.1.0 (2026-10-16)

The first release, for Linux x86-64: TradingView-style charts and drawings,
TradingView's built-in indicators, Pine Script®-compatible scripting (plus
Python, Luau, JavaScript and TypeScript), backtests with TradingView's Strategy
Report, a strategy lab, alerts, paper trading, a screener, and a Claude panel.
