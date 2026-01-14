<template>
  <div class="transaction-detail-page">
    <div v-if="loading" class="loading">
      <p>트랜잭션 조회 중...</p>
      <p v-if="isEthHash" class="info-text">
        Ethereum 트랜잭션 해시를 NetCoin 트랜잭션으로 변환 중입니다...
      </p>
    </div>
    <div v-else-if="error" class="error-container">
      <h2>❌ 트랜잭션을 찾을 수 없습니다</h2>
      <p class="error-message">{{ error }}</p>
      <p class="hash-display">
        검색한 해시: <code>{{ searchHash }}</code>
      </p>
      <div class="actions">
        <button @click="goToTransactions" class="btn btn-primary">
          모든 트랜잭션 보기
        </button>
      </div>
    </div>
    <div v-else-if="transaction" class="detail-container">
      <h1>트랜잭션 상세</h1>

      <div v-if="isEthHash" class="info-banner">
        ℹ️ 이 트랜잭션은 MetaMask를 통해 전송되었습니다
      </div>

      <div class="detail-grid">
        <div class="detail-item">
          <span class="label">해시</span>
          <span class="value monospace">{{ transaction.hash }}</span>
        </div>
        <div class="detail-item">
          <span class="label">보낸 주소</span>
          <span
            class="value address-link"
            @click="goToAddress(transaction.from)"
          >
            {{ transaction.from }}
          </span>
        </div>
        <div class="detail-item">
          <span class="label">받는 주소</span>
          <span class="value address-link" @click="goToAddress(transaction.to)">
            {{ transaction.to }}
          </span>
        </div>
        <div class="detail-item">
          <span class="label">금액</span>
          <span class="value amount"
            >{{ formatAmount(transaction.amount) }} NTC</span
          >
        </div>
        <div class="detail-item">
          <span class="label">수수료</span>
          <span class="value fee">{{ formatAmount(transaction.fee) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">총액</span>
          <span class="value total"
            >{{ formatTotal(transaction.amount, transaction.fee) }} NTC</span
          >
        </div>
        <div class="detail-item">
          <span class="label">상태</span>
          <span class="value status" :class="transaction.status">
            {{ transaction.status }}
          </span>
        </div>
        <div class="detail-item">
          <span class="label">블록 높이</span>
          <span class="value">
            {{
              transaction.block_height
                ? "#" + transaction.block_height
                : "대기 중"
            }}
          </span>
        </div>
        <div class="detail-item">
          <span class="label">생성 시간</span>
          <span class="value">{{ formatTime(transaction.timestamp) }}</span>
        </div>
      </div>

      <div class="actions">
        <button @click="goToTransactions" class="btn btn-primary">
          모든 트랜잭션 보기
        </button>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "TransactionDetail",
  data() {
    return {
      transaction: null,
      loading: false,
      error: null,
      searchHash: "",
      isEthHash: false,
    };
  },
  mounted() {
    this.fetchTransaction();
  },
  methods: {
    async fetchTransaction() {
      this.loading = true;
      this.error = null;

      try {
        const hash = this.$route.params.hash;
        this.searchHash = hash;
        this.isEthHash = hash.startsWith("0x");

        console.log("Fetching transaction:", hash);
        const res = await explorerAPI.getTransactionByHash(hash);
        this.transaction = res.data;
        console.log("Transaction loaded:", this.transaction);
      } catch (error) {
        console.error("트랜잭션 로딩 실패:", error);
        this.error =
          error.response?.data?.error || "트랜잭션을 찾을 수 없습니다.";
      } finally {
        this.loading = false;
      }
    },
    formatTime(timestamp) {
      const date = new Date(timestamp);
      return date.toLocaleString("ko-KR");
    },
    formatAmount(value) {
      // Handle hex string (0x...), decimal string, number, and U256 array format
      let num;

      if (Array.isArray(value)) {
        num =
          BigInt(value[0]) +
          (BigInt(value[1]) << BigInt(64)) +
          (BigInt(value[2]) << BigInt(128)) +
          (BigInt(value[3]) << BigInt(192));
      } else if (typeof value === "string") {
        if (value.startsWith("0x")) {
          num = BigInt(value);
        } else {
          num = BigInt(value);
        }
      } else {
        num = BigInt(value || 0);
      }

      const divisor = BigInt("1000000000000000000"); // 10^18
      const ntc = Number(num) / Number(divisor);

      return ntc.toLocaleString("en-US", {
        minimumFractionDigits: 0,
        maximumFractionDigits: 18,
      });
    },
    formatTotal(amount, fee) {
      // Convert both values using the same logic as formatAmount
      let numAmount, numFee;

      // Parse amount
      if (Array.isArray(amount)) {
        numAmount =
          BigInt(amount[0]) +
          (BigInt(amount[1]) << BigInt(64)) +
          (BigInt(amount[2]) << BigInt(128)) +
          (BigInt(amount[3]) << BigInt(192));
      } else if (typeof amount === "string" && amount.startsWith("0x")) {
        numAmount = BigInt(amount);
      } else {
        numAmount = BigInt(amount || 0);
      }

      // Parse fee
      if (Array.isArray(fee)) {
        numFee =
          BigInt(fee[0]) +
          (BigInt(fee[1]) << BigInt(64)) +
          (BigInt(fee[2]) << BigInt(128)) +
          (BigInt(fee[3]) << BigInt(192));
      } else if (typeof fee === "string" && fee.startsWith("0x")) {
        numFee = BigInt(fee);
      } else {
        numFee = BigInt(fee || 0);
      }

      const total = numAmount + numFee;
      const divisor = BigInt("1000000000000000000"); // 10^18
      const ntc = Number(total) / Number(divisor);

      return ntc.toLocaleString("en-US", {
        minimumFractionDigits: 0,
        maximumFractionDigits: 18,
      });
    },
    goToTransactions() {
      this.$router.push("/transactions");
    },
    goToAddress(address) {
      this.$router.push(`/address/${address}`);
    },
  },
};
</script>

<style scoped>
.transaction-detail-page {
  background: white;
  padding: 2rem;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}

h1 {
  margin-bottom: 2rem;
  color: #667eea;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: 2rem;
  margin-bottom: 2rem;
}

.detail-item {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 1rem;
  background-color: #f8f9ff;
  border-radius: 8px;
  border-left: 4px solid #667eea;
}

.label {
  font-size: 0.9rem;
  color: #666;
  font-weight: bold;
}

.value {
  word-break: break-all;
  color: #333;
}

.monospace {
  font-family: "Courier New", monospace;
  font-size: 0.85rem;
}

.address-link {
  cursor: pointer;
  color: #667eea;
  text-decoration: underline;
  transition: color 0.3s;
}

.address-link:hover {
  color: #764ba2;
}

.amount {
  color: #10b981;
  font-weight: bold;
}

.fee {
  color: #f59e0b;
  font-weight: bold;
}

.total {
  color: #667eea;
  font-weight: bold;
}

.status {
  font-weight: bold;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
  display: inline-block;
}

.status.confirmed {
  color: #10b981;
  background-color: #d1fae5;
}

.status.pending {
  color: #f59e0b;
  background-color: #fef3c7;
}

.actions {
  display: flex;
  gap: 1rem;
}

.btn {
  padding: 0.75rem 1.5rem;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-weight: bold;
  transition: all 0.3s;
}

.btn-primary {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
}

.btn-primary:hover {
  transform: scale(1.05);
}

.loading {
  text-align: center;
  padding: 3rem;
  color: #999;
}

.loading .info-text {
  margin-top: 1rem;
  color: #667eea;
  font-size: 0.9rem;
}

.error-container {
  padding: 2rem;
  text-align: center;
}

.error-container h2 {
  color: #ef4444;
  margin-bottom: 1rem;
}

.error-message {
  color: #666;
  margin-bottom: 1rem;
}

.hash-display {
  background-color: #f8f9ff;
  padding: 1rem;
  border-radius: 8px;
  margin: 1rem 0;
  word-break: break-all;
}

.hash-display code {
  font-family: "Courier New", monospace;
  color: #667eea;
  font-size: 0.9rem;
}

.info-banner {
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  padding: 1rem;
  border-radius: 8px;
  margin-bottom: 2rem;
  text-align: center;
  font-weight: bold;
}
</style>
