# Netcoin DNS Server

Netcoin 네트워크의 노드 디스커버리를 위한 DNS 서버입니다.

## 기능

- 노드 등록 및 관리
- 노드 목록 조회
- 자동 오래된 노드 정리
- 노드 통계 제공

## 빌드 및 실행

### DNS 서버 실행

```bash
cd dns-server
cargo run
```

또는 포트와 최대 노드 유효 시간을 지정:

```bash
cargo run -- --port 8053 --max-age 3600
```

### 옵션

- `--port` 또는 `-p`: DNS 서버 포트 (기본값: 8053)
- `--max-age` 또는 `-m`: 노드의 최대 유효 시간 (초 단위, 기본값: 3600)

## API 엔드포인트

### 1. 노드 등록

**POST** `/register`

노드를 DNS 서버에 등록합니다.

**요청 본문:**

```json
{
  "address": "192.168.1.100",
  "port": 8333,
  "version": "0.1.0",
  "height": 12345
}
```

**응답:**

```json
{
  "success": true,
  "message": "Node 192.168.1.100:8333 registered successfully",
  "node_count": 42
}
```

### 2. 노드 목록 조회

**GET** `/nodes?limit=10&min_height=1000`

등록된 노드 목록을 조회합니다.

**쿼리 파라미터:**

- `limit` (선택): 반환할 최대 노드 수
- `min_height` (선택): 최소 블록 높이

**응답:**

```json
{
  "nodes": [
    {
      "address": "192.168.1.100",
      "port": 8333,
      "version": "0.1.0",
      "height": 12345,
      "last_seen": 1737327600
    }
  ],
  "count": 1
}
```

### 3. 서버 상태 확인

**GET** `/health`

서버의 상태를 확인합니다.

**응답:**

```json
{
  "status": "healthy",
  "node_count": 42,
  "timestamp": 1737327600
}
```

### 4. 통계 조회

**GET** `/stats`

네트워크 통계를 조회합니다.

**응답:**

```json
{
  "node_count": 42,
  "max_height": 12500,
  "avg_height": 12000,
  "versions": {
    "0.1.0": 30,
    "0.1.1": 12
  },
  "timestamp": 1737327600
}
```

## P2P 노드에서 DNS 사용하기

### 1. DNS 서버에 노드 등록

```rust
use std::sync::Arc;

// PeerManager 생성 후
let peer_manager = Arc::new(PeerManager::new());

// DNS 서버에 등록
let dns_server = "http://dns.netcoin.org:8053";
let my_address = "192.168.1.100"; // 외부에서 접근 가능한 주소
let my_port = 8333;

peer_manager
    .register_with_dns(dns_server, my_address, my_port)
    .await?;
```

### 2. 주기적으로 DNS에 등록 (백그라운드)

```rust
// 5분마다 DNS에 등록
let peer_manager_clone = peer_manager.clone();
tokio::spawn(async move {
    peer_manager_clone
        .start_dns_registration_loop(
            "http://dns.netcoin.org:8053".to_string(),
            "192.168.1.100".to_string(),
            8333,
            300, // 5분마다
        )
        .await;
});
```

### 3. DNS에서 피어 목록 가져오기

```rust
// 최대 20개의 노드를 가져옴 (최소 높이 1000 이상)
let peers = peer_manager
    .fetch_peers_from_dns(
        "http://dns.netcoin.org:8053",
        Some(20),
        Some(1000),
    )
    .await?;

// 가져온 피어들에 연결
for peer_addr in peers {
    let pm = peer_manager.clone();
    tokio::spawn(async move {
        if let Err(e) = pm.connect_peer(&peer_addr).await {
            log::warn!("Failed to connect to {}: {:?}", peer_addr, e);
        }
    });
}
```

## 예제: 전체 노드 실행 흐름

```rust
use std::sync::Arc;
use netcoin_node::p2p::PeerManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let peer_manager = Arc::new(PeerManager::new());

    // 1. DNS에서 피어 목록 가져오기
    let dns_server = "http://dns.netcoin.org:8053";
    let peers = peer_manager
        .fetch_peers_from_dns(dns_server, Some(10), None)
        .await?;

    // 2. 가져온 피어들에 연결
    for peer_addr in peers {
        let pm = peer_manager.clone();
        tokio::spawn(async move {
            let _ = pm.connect_peer(&peer_addr).await;
        });
    }

    // 3. P2P 리스너 시작
    let pm = peer_manager.clone();
    tokio::spawn(async move {
        pm.start_listener("0.0.0.0:8333").await.unwrap();
    });

    // 4. DNS 등록 시작 (5분마다)
    let pm = peer_manager.clone();
    tokio::spawn(async move {
        pm.start_dns_registration_loop(
            dns_server.to_string(),
            "your.public.ip".to_string(),
            8333,
            300,
        ).await;
    });

    // 메인 루프...
    tokio::signal::ctrl_c().await?;

    Ok(())
}
```

## Docker로 DNS 서버 실행

### Dockerfile

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY dns-server/Cargo.toml dns-server/Cargo.lock ./
COPY dns-server/src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/netcoin-dns /usr/local/bin/
EXPOSE 8053
CMD ["netcoin-dns", "--port", "8053"]
```

### Docker Compose

```yaml
version: "3.8"

services:
  dns-server:
    build:
      context: .
      dockerfile: dns-server/Dockerfile
    ports:
      - "8053:8053"
    environment:
      - RUST_LOG=info
    restart: unless-stopped
```

실행:

```bash
docker-compose up -d dns-server
```

## 보안 고려사항

1. **DDoS 방지**: 프로덕션 환경에서는 rate limiting 추가 권장
2. **인증**: 필요시 API 키 기반 인증 추가
3. **HTTPS**: 프로덕션에서는 리버스 프록시(nginx, caddy)를 통한 HTTPS 사용 권장
4. **노드 검증**: 등록된 노드의 실제 접근 가능 여부 검증 로직 추가 고려

## 모니터링

DNS 서버 상태는 `/health` 엔드포인트로 모니터링할 수 있습니다:

```bash
curl http://localhost:8053/health
```

통계 확인:

```bash
curl http://localhost:8053/stats
```

## 라이선스

MIT
