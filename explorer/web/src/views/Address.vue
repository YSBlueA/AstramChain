<template>
  <div class="address-page">
    <div v-if="addressInfo" class="detail-container">
      <h1>주소 정보</h1>

      <div class="address-header">
        <div class="address-hash monospace">{{ addressInfo.address }}</div>
      </div>

      <div class="detail-grid">
        <div class="detail-item highlight">
          <span class="label">잔액</span>
          <span class="value balance">{{ formatAmount(addressInfo.balance) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">받은 금액</span>
          <span class="value received">{{ formatAmount(addressInfo.received) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">보낸 금액</span>
          <span class="value sent">{{ formatAmount(addressInfo.sent) }} NTC</span>
        </div>
        <div class="detail-item">
          <span class="label">트랜잭션 수</span>
          <span class="value">{{ addressInfo.transaction_count }}</span>
        </div>
        <div class="detail-item">
          <span class="label">마지막 활동</span>
          <span class="value">
            {{
              addressInfo.last_transaction
                ? formatTime(addressInfo.last_transaction)
                : "활동 없음"
            }}
          </span>
        </div>
      </div>

      <div class="actions">
        <button @click="goHome" class="btn btn-primary">홈으로</button>
      </div>
    </div>
    <div v-else class="loading">로딩 중...</div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "Address",
  data() {
    return {
      addressInfo: null,
    };
  },
  mounted() {
    this.fetchAddressInfo();
  },
  methods: {
    async fetchAddressInfo() {
      try {
        const address = this.$route.params.address;
        const res = await explorerAPI.getAddressInfo(address);
        this.addressInfo = res.data;
      } catch (error) {
        console.error("주소 정보 로딩 실패:", error);
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
    goHome() {
      this.$router.push("/");
    },
  },
};
</script>

<style scoped>
.address-page {
  background: white;
  padding: 2rem;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}

h1 {
  margin-bottom: 2rem;
  color: #667eea;
}

.address-header {
  background-color: #f8f9ff;
  padding: 1.5rem;
  border-radius: 8px;
  margin-bottom: 2rem;
  border-left: 4px solid #667eea;
}

.address-hash {
  font-size: 0.9rem;
  word-break: break-all;
  color: #333;
}

.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
  gap: 2rem;
  margin-bottom: 2rem;
}

.detail-item {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 1.5rem;
  background-color: #f8f9ff;
  border-radius: 8px;
  border-left: 4px solid #ddd;
}

.detail-item.highlight {
  border-left-color: #667eea;
  background: linear-gradient(135deg, #f8f9ff 0%, #f0f3ff 100%);
}

.label {
  font-size: 0.9rem;
  color: #666;
  font-weight: bold;
}

.value {
  font-size: 1.1rem;
  color: #333;
}

.balance {
  color: #667eea;
  font-weight: bold;
}

.received {
  color: #10b981;
  font-weight: bold;
}

.sent {
  color: #f59e0b;
  font-weight: bold;
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
