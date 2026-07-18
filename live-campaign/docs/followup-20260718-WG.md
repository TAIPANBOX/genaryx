# Ф4 дозапуск 2026-07-18: WG-транспорт через застосунок + підписаний kill

Мета: закрити дві "зірочки" кампанії 2026-07-17, де канал був ручний `ssh -L`, а не
WG-транспорт самого застосунку, і не було задокументованого підписаного kill по
віддаленому Cloud.

## РЕЗУЛЬТАТ: EXIT-GATE PASSED (2026-07-18)

- WG-тунель піднято САМИМ застосунком (native SwiftUI під root): keypair -> обмін ключами з боксом -> `wireguard-go` -> handshake -> ~28 MB через `utun6`. Не `ssh -L`.
- Усі 3 плани досяжні ТІЛЬКИ через тунель (`10.9.0.1:8080/8090/8081`): money $4,314, policy 6, identity 29.
- Control plane закритий з інтернету (`ufw`: публічно лише `:22`+`:51820/udp`; `:8080` ззовні timeout).
- Hardware-signed kill (Touch ID + ES256 break-glass + причина оператора) runaway `cashflow-forecaster-0217` пройшов ЧЕРЕЗ WG-тунель -> `killed=true` на боксі, ACTIVE RUNS 9,289 -> 9,288.
- Обидві "зірочки" 2026-07-17 закриті (канал = WG-транспорт застосунку; підписаний kill через цей тунель).
- **Баг знайдено + фікшено:** `wg.rs set_addr` на macOS використовував `ifconfig ... alias` (тихо не призначає IP на utun) -> замінено на `netmask 255.255.255.255`; тест оновлено; connectors 98/98.
- **Feature gap (не блокує):** застосунок фіксує money-дескриптор при старті; переключення планів на активний тунель без рестарту = майбутній чистий фікс (у кампанії обійдено loopback->WG форвардом).

## Стан бокса (готово, зроблено автономно)

- Бокс: Hetzner CPX62, `5.75.234.176` (Hetzner перевидав той самий IP, що 07-17; хост-ключ
  запінено в новий файл `~/.ssh/known_hosts_genaryx_followup_20260718`).
- SSH-ключ (свіжий): `~/.ssh/hetzner-genaryx-20260718` (старий 07-17 не чіпав).
- Стек через `stack-up`: Cloud `0.0.0.0:8080`, wardryx/idryx/gateway на `127.0.0.1`.
- Плани засіяні (числа детерміновано збігаються з 07-17):
  - money: $4,314.54 spent, $2,370.40 prevented, $2,992.70 saved, 180 breaks, 9,289 runs, 34,834 calls, 176 incidents.
  - policy: 6 політик + 5 pending approvals.
  - identity: meridian idryx на `127.0.0.1:8082`, 29 identities, 44 alerts.

## WG-сервер (kernel wg-quick на боксі)

- `wg0` up: server addr `10.9.0.1/24`, listen `:51820`.
- Server pubkey: `4OhTOyJS92ml7CTrXxio1ziAPc+9m5CtpPPtHYOog2U=`
- Endpoint: `5.75.234.176:51820`
- socat-форварди на wg0 (бо сервіси localhost-only): `10.9.0.1:8090 -> 127.0.0.1:8090`,
  `10.9.0.1:8081 -> 127.0.0.1:8082` (meridian idryx), `10.9.0.1:4100 -> 127.0.0.1:4100`.
  Cloud уже на `0.0.0.0`, тож видно на `10.9.0.1:8080` через тунель.
- ufw: публічно ТІЛЬКИ `22/tcp` + `51820/udp`; весь control plane закритий ззовні
  (перевірено: `5.75.234.176:8080` дає timeout). Це і є вимога D11 "not exposed to internet".

## Значення для Remote-панелі застосунку (Mac-клієнт)

- WG peer pubkey (server): `4OhTOyJS92ml7CTrXxio1ziAPc+9m5CtpPPtHYOog2U=`
- WG endpoint: `5.75.234.176:51820`
- allowed-ips (що маршрутизувати в тунель): `10.9.0.0/24`
- local (tunnel) address: `10.9.0.2/32`
- peer (tunnel) address: `10.9.0.1`
- keepalive: `25`
- wireguard-go bin: `~/.taipan/bin/wireguard-go` (встановлено локально, конектор знайде сам)

Дескриптор сервісів (через тунель): cloud `http://10.9.0.1:8080`, wardryx `http://10.9.0.1:8090`,
idryx `http://10.9.0.1:8081`, gateway `http://10.9.0.1:4100` (enforce).

## Крок, що потребує оператора (root на Mac)

Підняття utun на macOS потребує root. У застосунку `WgTunnel::bring_up` спавнить
`wireguard-go`, якому для tun-девайса потрібні привілеї. Варіанти для Юрія:
1. Запустити застосунок з-під `sudo` (одноразово, для демо), або
2. Заздалегідь дати `wireguard-go` право створювати utun, або
3. У терміналі один раз ввести пароль sudo, коли застосунок його запросить.

Після того, як застосунок згенерує console keypair і покаже console pubkey, додати його
як peer на боксі (я зроблю сам, щойно матиму pubkey):

```
ssh -i ~/.ssh/hetzner-genaryx-20260718 -o UserKnownHostsFile=~/.ssh/known_hosts_genaryx_followup_20260718 \
  root@5.75.234.176 'wg set wg0 peer <CONSOLE_PUBKEY> allowed-ips 10.9.0.2/32 && wg show wg0'
```

## Стан на кінець автономної сесії (2026-07-18)

Зроблено самостійно:
- Бокс піднято, 3 плани засіяно (числа збігаються), WG-сервер + socat-форварди + ufw (control plane закритий ззовні, перевірено).
- UI-редизайн: усі 14 вкладок обох шелів на дашборд-форму + `FreshBadge` (LIVE/AUTO/SNAPSHOT/ON-DEMAND/WINDOW/PAUSED). Гейти особисто перезапущені й зелені: Tauri `tsc --noEmit` + `pnpm build`, SwiftUI `swift build` (усі 45 файлів). Моделі мутацій (kill/budget/grant/deny/forget) НЕ змінені, тільки View-шари; Touch ID на місці; Identity 20s-loop прибраний (parity fix). Робота НЕ закомічена.
- Для перегляду вигляду з реальними даними: підняті persistent SSH-форварди (8080/8090/8081->8082/4100, root не треба) і перезапущено нативний `Genaryx.app` (pid підхоплює дані через форвард). Це showcase вигляду, НЕ WG exit-gate.

Чекає оператора (root/присутність):
- computer-use до Genaryx відхилено (`user_denied`), тож нативні скріншоти нових вкладок автономно не зробити.
- WG-тунель через Remote-панель + підписаний kill: підняття utun на Mac потребує root (пароль sudo), вводити який агенту заборонено.

## Підписаний kill (exit-gate)

Кандидат: живий ран `cashflow-forecaster-0217` (~$5.85, найбільший незабитий на момент
перевірки). Через застосунок: Money -> рядок рану -> Kill -> break-glass причина
(SwiftUI: + Touch ID) -> ES256-підписаний `money_kill_run` іде В ТУНЕЛІ на `10.9.0.1:8080`.
Це закриває "hardware-signed kill проти віддаленого client-hosted Cloud через
Genaryx-транспорт". Далі verify, що ран killed, і скрін.
