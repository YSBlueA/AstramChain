<template>
  <div class="transaction-detail-page">
    <div v-if="transaction" class="detail-container">
      <h1>트랜잭션 상세</h1>

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
          <span class="value amount">{{ formatAmount(transaction.amount) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">수수료</span>
          <span class="value fee">{{ formatAmount(transaction.fee) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">총액</span>
          <span class="value total">{{ formatAmount(transaction.amount + transaction.fee) }} NTC</span>
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
    <div v-else class="loading">로딩 중...</div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "TransactionDetail",
  data() {
    return {
      transaction: null,
    };
  },
  mounted() {
    this.fetchTransaction();
  },
  methods: {
    async fetchTransaction() {
      try {
        const hash = this.$route.params.hash;
        const res = await explorerAPI.getTransactionByHash(hash);
        this.transaction = res.data;
      } catch (error) {
        console.error("트랜잭션 로딩 실패:", error);
      }
    },
    formatTime(timestamp) {
      const date = new Date(timestamp);
      return date.toLocaleString("ko-KR");
    },
    formatAmount(value) {
      const num = Number(value) || 0;
      const ntc = num / 100_000_000; // Convert natoshi to NTC
      return ntc.toLocaleString("en-US", {
        minimumFractionDigits: 0,
        maximumFractionDigits: 8,
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
</style>
