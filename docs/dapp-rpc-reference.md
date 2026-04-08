# AstramChain dApp RPC Reference

dApp에서 AstramChain 노드와 직접 통신하거나, Chrome 확장 지갑(`window.astramWallet`)을 통해 트랜잭션을 처리하는 방법을 설명합니다.

---

## 목차

1. [기본 정보](#1-기본-정보)
2. [단위 체계](#2-단위-체계)
3. [Public RPC 엔드포인트](#3-public-rpc-엔드포인트)
4. [트랜잭션 제출](#4-트랜잭션-제출)
5. [AstramX Wallet API (window.astramWallet)](#5-astramx-wallet-api)
6. [수수료 계산](#6-수수료-계산)
7. [에러 처리](#7-에러-처리)

---

## 1. 기본 정보

| 항목 | 값 |
|------|-----|
| Public RPC URL | `https://rpc.astramchain.com` |
| 프로토콜 | HTTP REST (JSON 응답) |
| 트랜잭션 인코딩 | `bincode v2 standard` + `Base64` |
| 블록 목표 시간 | 120초 |
| 합의 알고리즘 | Proof of Work (KawPow-Blake3) |

Public RPC 서버는 읽기 전용 엔드포인트와 트랜잭션 제출만 허용합니다.  
대시보드, 마이닝, 내부 블록체인 메모리 접근 등은 노출되지 않습니다.

---

## 2. 단위 체계

AstramChain은 18자리 소수점을 사용합니다.

| 단위 | 값 | 설명 |
|------|----|------|
| `ram` | `1` | 최소 단위 (wei에 해당) |
| `ASRM` | `10^18 ram` | 표시 단위 |

### 변환 예시

```js
// ASRM → ram
const amountRam = BigInt(Math.round(amountAsrm * 1e18))

// ram → ASRM
const amountAsrm = Number(amountRam) / 1e18
```

### 금액 필드 형식

API 응답의 금액은 두 가지 형식으로 올 수 있습니다:

- **`balance` 필드**: U256 정수 (10진수 문자열 또는 `0x` 헥스)
- **UTXO `amount` 필드**: `[u64, u64, u64, u64]` 형태의 little-endian 4개 워드 배열

```js
// UTXO amount 워드 배열 → BigInt 변환
function wordsToAmount(words) {
  return (
    BigInt(words[0]) |
    (BigInt(words[1]) << 64n) |
    (BigInt(words[2]) << 128n) |
    (BigInt(words[3]) << 192n)
  )
}
```

---

## 3. Public RPC 엔드포인트

### GET /health

노드 상태 확인.

```http
GET /health
```

**응답**
```json
{
  "status": "ok",
  "height": 48321,
  "timestamp": 1712500000
}
```

---

### GET /status

노드·네트워크·마이닝 전체 현황.

```http
GET /status
```

**응답**
```json
{
  "node": { "version": "0.5.0" },
  "blockchain": {
    "height": 48321,
    "memory_blocks": 128,
    "chain_tip": "a3f8c2...",
    "difficulty": 305441741
  },
  "mempool": {
    "pending_transactions": 3,
    "seen_transactions": 41
  },
  "network": {
    "network_id": "Astram-mainnet",
    "chain_id": 1,
    "network_magic": "0xd9b4bef9",
    "connected_peers": 7,
    "peer_heights": { "192.168.1.1:9333": 48320 },
    "subnet_diversity": {
      "unique_24_subnets": 5,
      "unique_16_subnets": 4
    }
  },
  "mining": {
    "active": true,
    "hashrate": 1540000000,
    "difficulty": 305441741
  },
  "security": {
    "validation_failures_total": 0,
    "validation_failures": []
  },
  "timestamp": "2026-04-08T10:00:00Z"
}
```

---

### GET /counts

블록·트랜잭션 수 및 총 거래량 요약.

```http
GET /counts
```

**응답**
```json
{
  "blocks": 48321,
  "transactions": 192840,
  "total_volume": "0x1bc16d674ec80000"
}
```

---

### GET /address/{address}/balance

주소의 현재 잔액 조회. 주소는 소문자 hex 형식.

```http
GET /address/ab12cd34.../balance
```

**응답**
```json
{
  "address": "ab12cd34...",
  "balance": 5000000000000000000
}
```

> `balance` 값은 ram 단위 정수입니다. ASRM으로 변환: `balance / 1e18`

---

### GET /address/{address}/info

주소의 상세 통계 (잔액, 총 수신/송신, 트랜잭션 수).

```http
GET /address/ab12cd34.../info
```

**응답**
```json
{
  "address": "ab12cd34...",
  "balance": "0x4563918244f40000",
  "received": "0x8ac7230489e80000",
  "sent": "0x45639182cb700000",
  "transaction_count": 12
}
```

> `balance`, `received`, `sent`는 `0x` 헥스 형식의 ram 단위입니다.

---

### GET /address/{address}/utxos

주소의 미사용 트랜잭션 출력(UTXO) 목록.

```http
GET /address/ab12cd34.../utxos
```

**응답** (배열)
```json
[
  {
    "txid": "f3a8c2d1...",
    "vout": 0,
    "amount": [5000000000000000000, 0, 0, 0]
  }
]
```

> `amount`는 `[u64; 4]` little-endian 워드 배열입니다. `wordsToAmount()` 함수로 변환하세요.

---

### GET /address/{address}/transactions

주소의 트랜잭션 내역. 최신순 정렬.

```http
GET /address/ab12cd34.../transactions?limit=20
```

| 파라미터 | 타입 | 설명 |
|----------|------|------|
| `limit` | `number` (optional) | 최대 반환 건수 (기본값: 전체) |

**응답**
```json
{
  "address": "ab12cd34...",
  "transactions": [
    {
      "txid": "f3a8c2d1...",
      "block_height": 48300,
      "timestamp": 1712499000,
      "direction": "send",
      "amount": "5000000000000000000",
      "counterpart": "9f1e2a3b..."
    }
  ]
}
```

| 필드 | 설명 |
|------|------|
| `direction` | `"send"` 또는 `"receive"` |
| `amount` | ram 단위 문자열 |
| `counterpart` | 상대방 주소 |
| `timestamp` | Unix timestamp (초) |

---

### GET /tx/{txid}

트랜잭션 상세 조회.

```http
GET /tx/f3a8c2d1...
```

**응답**
```json
{
  "txid": "f3a8c2d1...",
  "block_height": 48300,
  "transaction": "<bincode+base64 encoded>",
  "encoding": "bincode+base64"
}
```

> `transaction` 필드는 bincode v2로 직렬화된 뒤 Base64로 인코딩된 바이너리입니다.  
> 일반 dApp에서는 이 필드를 직접 파싱할 필요가 없습니다.

**404 응답**
```json
{ "error": "tx not found" }
```

---

### GET /blockchain/range

특정 블록 높이 구간의 블록 조회.

```http
GET /blockchain/range?from=100&to=110
```

| 파라미터 | 타입 | 설명 |
|----------|------|------|
| `from` | `number` | 시작 높이 (포함, 기본값: 0) |
| `to` | `number` (optional) | 종료 높이 (포함) |

**응답**
```json
{
  "blockchain": "<bincode+base64 encoded blocks>",
  "count": 11,
  "from": 100,
  "to": 110,
  "source": "database"
}
```

---

## 4. 트랜잭션 제출

### POST /tx

서명된 트랜잭션을 네트워크에 브로드캐스트합니다.

```http
POST /tx
Content-Type: application/octet-stream

<bincode v2로 직렬화된 트랜잭션 바이너리>
```

**성공 응답** (`200 OK`)
```json
{ "status": "ok", "message": "tx queued" }
```

**중복 트랜잭션** (`200 OK`)
```json
{ "status": "duplicate" }
```

**오류 응답** (`400 Bad Request`)
```json
{ "status": "error", "message": "fee too low: got 100000 ram, need 160000000000000 ram" }
```

### 트랜잭션 직렬화 (JavaScript)

AstramChain 트랜잭션은 bincode v2 standard 포맷으로 직렬화됩니다.  
`chrome-wallet/src/utils/transaction.ts`의 `sendTransaction()` 함수가 전체 흐름을 구현합니다:

1. UTXOs 조회 (`GET /address/{from}/utxos`)
2. 코인 선택 및 거스름돈 계산
3. `serialize_for_hash()` → SHA-256 double hash → 서명 (Ed25519, tweetnacl)
4. `serializeTransaction()` → 바이너리 → `POST /tx`

---

## 5. AstramX Wallet API

`window.astramWallet`은 Chrome 확장 지갑이 페이지에 주입하는 JavaScript 객체입니다.  
dApp은 이 API를 통해 사용자 서명과 트랜잭션 제출을 요청할 수 있습니다.

### 지갑 감지

```js
if (!window.astramWallet) {
  alert('AstramX Wallet 확장 프로그램을 설치하세요.')
}
```

---

### getBalance(address)

특정 주소의 잔액을 조회합니다.

```ts
const balance = await window.astramWallet.getBalance(address)
// 반환값: ram 단위 정수 (string 또는 number)
```

---

### signTransaction(tx)

트랜잭션 서명 및 브로드캐스트를 지갑에 요청합니다.  
사이드 패널에 승인 UI가 표시되며, 사용자가 승인하면 자동으로 네트워크에 제출됩니다.

```ts
interface TxRequest {
  to: string    // 수신 주소 (hex)
  amount: number // ASRM 단위 (예: 1.5)
}

interface TxResult {
  hash: string  // 트랜잭션 ID (txid)
}

const result = await window.astramWallet.signTransaction({ to, amount })
console.log('txid:', result.hash)
```

**실패 시 예외 발생:**
- `"User rejected the transaction"` — 사용자가 거절
- `"Another transaction is pending approval"` — 이미 승인 대기 중인 요청 존재
- `"Transaction approval timed out"` — 5분 내 응답 없음
- `"잔액 부족: ..."` — UTXO 합계 부족

#### 전체 dApp 예시

```ts
async function sendPayment(to: string, amount: number): Promise<string> {
  if (!window.astramWallet) {
    throw new Error('AstramX Wallet not installed')
  }

  try {
    const result = await window.astramWallet.signTransaction({ to, amount })
    return result.hash
  } catch (err) {
    if (err.message === 'User rejected the transaction') {
      // 사용자가 거절한 경우 조용히 처리
      return ''
    }
    throw err
  }
}
```

#### 내부 메시지 흐름

```
dApp
 └─ window.astramWallet.signTransaction()    [inject.js]
      └─ postMessage(ASTRAM_REQUEST)
           └─ content-script.ts
                └─ chrome.runtime.sendMessage(WALLET_REQUEST)
                     └─ service-worker.ts
                          ├─ pendingTx → storage 저장
                          ├─ 사이드패널 열기
                          └─ storage 변화 감지 대기
                               └─ TxApproval UI (사용자 승인/거절)
                                    ├─ 승인: sendTransaction() → POST /tx → txResult 저장
                                    └─ 거절: txResult { approved: false } 저장
                                         └─ dApp에 결과 반환
```

---

## 6. 수수료 계산

수수료는 트랜잭션 바이트 크기에 따라 결정됩니다.

| 항목 | 값 |
|------|-----|
| 기본 수수료 | `100,000,000,000,000 ram` (= 0.0001 ASRM) |
| 릴레이 최소 수수료 (바이트당) | `200,000,000,000 ram` (= 200 Gwei) |
| 기본 지갑 수수료 (바이트당) | `300,000,000,000 ram` (= 300 Gwei) |

### 계산 공식

```
최소 수수료 = 100_000_000_000_000 + (tx_size_bytes × 200_000_000_000)
기본 수수료 = 100_000_000_000_000 + (tx_size_bytes × 300_000_000_000)
```

**예시** (트랜잭션 크기 300 bytes):
```
최소 수수료 = 0.0001 + (300 × 0.0000002) = 0.00016 ASRM
기본 수수료 = 0.0001 + (300 × 0.0000003) = 0.00019 ASRM
```

지갑은 기본 수수료(1.5x)를 사용하여 빠른 확인을 보장합니다.

---

## 7. 에러 처리

### HTTP 상태 코드

| 코드 | 의미 |
|------|------|
| `200 OK` | 성공 또는 중복 (`status: "duplicate"`) |
| `400 Bad Request` | 잘못된 요청 (서명 오류, 수수료 부족, double-spend 등) |
| `404 Not Found` | 트랜잭션/리소스 없음 |
| `500 Internal Server Error` | 서버 내부 오류 |

### 공통 에러 응답 형식

```json
{ "error": "에러 메시지" }
{ "status": "error", "message": "에러 메시지" }
```

### wallet API 에러 처리

```ts
try {
  const result = await window.astramWallet.signTransaction({ to, amount })
} catch (e) {
  switch (e.message) {
    case 'AstramX Wallet not installed':
      // 설치 유도
      break
    case 'User rejected the transaction':
      // 사용자가 취소함 — 정상 케이스
      break
    case 'Another transaction is pending approval':
      // 이미 진행 중인 요청 있음
      break
    case 'Transaction approval timed out':
      // 5분 초과
      break
    default:
      // 잔액 부족, 네트워크 오류 등
      console.error(e.message)
  }
}
```
