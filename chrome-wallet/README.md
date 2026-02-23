# Astram Chrome Wallet Extension

Chrome 확장으로 연동되는 Astram 블록체인 지갑입니다.

## 기능

- 🔐 Wallet Import & Management
- 💰 Balance Tracking
- 📤 Transaction Signing
- 🔗 Astram RPC 연동

## 설치

### 1. 의존성 설치
```bash
npm install
```

### 2. 빌드
```bash
npm run build
```

### 3. Chrome에 로드

1. Chrome 주소창에 `chrome://extensions` 입력
2. **개발자 모드** 활성화 (우상단)
3. **확장 프로그램 로드** 클릭
4. `chrome-wallet/dist` 디렉토리 선택

## 개발

```bash
npm run dev
```

## 디렉토리 구조

```
chrome-wallet/
├── src/
│   ├── components/        # React 컴포넌트
│   ├── background/        # Service Worker
│   ├── content/           # Content Script
│   ├── inject/            # Injected Script
│   ├── store/             # Zustand Store
│   ├── App.tsx           # 메인 앱
│   ├── popup.tsx         # Popup 진입점
│   └── popup.html        # Popup HTML
├── manifest.json         # Chrome Extension 매니페스트
├── package.json          # npm 패키지
├── vite.config.ts        # Vite 설정
└── tsconfig.json         # TypeScript 설정
```

## 사용 방법

### 지갑 가져오기

1. 확장 아이콘 클릭
2. "Import Wallet" 클릭
3. Address와 Private Key 입력
4. "Import" 클릭

### 잔액 확인

- 지갑 정보 화면에서 실시간으로 잔액 표시
- "Refresh" 버튼으로 수동 새로고침

### 웹사이트에서 사용

```javascript
// 지갑 연동된 웹사이트에서
const balance = await window.astramWallet.getBalance('0x...')
const signed = await window.astramWallet.signTransaction(tx)
```

## 설정

### Astram RPC 주소 변경

`src/components/WalletHome.tsx`에서:

```typescript
const ASTRAM_RPC = 'http://localhost:19533'  // 원하는 주소로 변경
```

## 라이선스

MIT
