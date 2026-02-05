<template>
  <div class="block-detail-page">
    <div v-if="block" class="detail-container">
      <h1>Block #{{ block.height }}</h1>

      <div class="detail-grid">
        <div class="detail-item">
          <span class="label">Hash</span>
          <span class="value monospace">{{ block.hash }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Confirmations</span>
          <span class="value" :class="getConfirmationClass(block.confirmations)">
            {{ block.confirmations }} {{ getConfirmationStatus(block.confirmations) }}
          </span>
        </div>
        <div class="detail-item">
          <span class="label">Previous Hash</span>
          <span class="value monospace">{{ block.previous_hash }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Miner</span>
          <span class="value">{{ block.miner }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Timestamp</span>
          <span class="value">{{ formatTime(block.timestamp) }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Transactions</span>
          <span class="value">{{ block.transactions }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Difficulty</span>
          <span class="value">{{ block.difficulty }}</span>
        </div>
        <div class="detail-item">
          <span class="label">Nonce</span>
          <span class="value">{{ block.nonce }}</span>
        </div>
      </div>

      <div class="actions">
        <button @click="goToBlocks" class="btn btn-primary">
          View All Blocks
        </button>
      </div>
    </div>
    <div v-else class="loading">Loading...</div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "BlockDetail",
  data() {
    return {
      block: null,
    };
  },
  mounted() {
    this.fetchBlock();
  },
  methods: {
    async fetchBlock() {
      try {
        const height = this.$route.params.height;
        const res = await explorerAPI.getBlockByHeight(height);
        this.block = res.data;
      } catch (error) {
        console.error("Failed to load block:", error);
      }
    },
    formatTime(timestamp) {
      const date = new Date(timestamp);
      return date.toLocaleString("ko-KR");
    },
    getConfirmationClass(confirmations) {
      if (confirmations === 0) return 'status-unconfirmed';
      if (confirmations < 6) return 'status-pending';
      return 'status-confirmed';
    },
    getConfirmationStatus(confirmations) {
      if (confirmations === 0) return '(Unconfirmed)';
      if (confirmations < 6) return '(Low Confidence)';
      return '(Confirmed)';
    },
    goToBlocks() {
      this.$router.push("/blocks");
    },
  },
};
</script>

<style scoped>
.block-detail-page {
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

.status-unconfirmed {
  color: #e74c3c;
  font-weight: bold;
}

.status-pending {
  color: #f39c12;
  font-weight: bold;
}

.status-confirmed {
  color: #27ae60;
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
