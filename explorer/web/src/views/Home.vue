<template>
  <div class="home-page">
    <div class="hero">
      <h1>⛏️ NetCoin Blockchain Explorer</h1>
      <p>실시간 블록체인 상태 모니터링</p>
    </div>

    <div class="search-box">
      <input
        v-model="searchQuery"
        type="text"
        placeholder="블록 높이, 트랜잭션 해시, 주소 검색..."
        @keyup.enter="handleSearch"
      />
      <button @click="handleSearch">검색</button>
    </div>

    <div v-if="stats" class="stats-grid">
      <div class="stat-card">
        <div class="stat-label">총 블록</div>
        <div class="stat-value">{{ stats.total_blocks }}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">총 트랜잭션</div>
        <div class="stat-value">{{ stats.total_transactions }}</div>
      </div>
      <div class="stat-card">
        <div class="stat-label">총 거래량</div>
        <div class="stat-value">
          {{ formatAmount(stats.total_volume) }}
        </div>
      </div>
      <div class="stat-card">
        <div class="stat-label">평균 블록 시간</div>
        <div class="stat-value">{{ stats.average_block_time.toFixed(1) }}s</div>
      </div>
    </div>

    <div class="recent-section">
      <div class="section">
        <h2>최근 블록</h2>
        <div v-if="recentBlocks.length" class="blocks-list">
          <div
            v-for="block in recentBlocks.slice(0, 5)"
            :key="block.hash"
            class="list-item"
            @click="goToBlock(block.height)"
          >
            <div class="item-header">
              <span class="block-height">#{{ block.height }}</span>
              <span class="timestamp">{{ formatTime(block.timestamp) }}</span>
            </div>
            <div class="item-detail">
              <span class="txs">{{ block.transactions }} txs</span>
              <span class="miner">{{ truncateAddress(block.miner) }}</span>
            </div>
          </div>
        </div>
        <div v-else class="loading">데이터 로딩 중...</div>
      </div>

      <div class="section">
        <h2>최근 트랜잭션</h2>
        <div v-if="recentTransactions.length" class="transactions-list">
          <div
            v-for="tx in recentTransactions.slice(0, 5)"
            :key="tx.hash"
            class="list-item"
            @click="goToTransaction(tx.hash)"
          >
            <div class="item-header">
              <span class="tx-hash">{{ truncateHash(tx.hash) }}</span>
              <span class="timestamp">{{ formatTime(tx.timestamp) }}</span>
            </div>
            <div class="item-detail">
              <span class="amount">{{ formatAmount(tx.amount) }} NTC</span>
              <span class="status" :class="tx.status">{{ tx.status }}</span>
            </div>
          </div>
        </div>
        <div v-else class="loading">데이터 로딩 중...</div>
      </div>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "Home",
  data() {
    return {
      searchQuery: "",
      stats: null,
      recentBlocks: [],
      recentTransactions: [],
    };
  },
  mounted() {
    this.fetchData();
    // 10초마다 데이터 새로고침
    setInterval(() => this.fetchData(), 10000);
  },
  methods: {
    async fetchData() {
      try {
        const [statsRes, blocksRes, txsRes] = await Promise.all([
          explorerAPI.getStats(),
          explorerAPI.getBlocks(1, 10),
          explorerAPI.getTransactions(1, 10),
        ]);

        this.stats = statsRes.data;
        this.recentBlocks = blocksRes.data.blocks || [];
        this.recentTransactions = txsRes.data.transactions || [];
      } catch (error) {
        console.error("데이터 로딩 실패:", error);
      }
    },
    handleSearch() {
      if (!this.searchQuery.trim()) return;

      const query = this.searchQuery.trim();

      // 높이로 검색 (숫자)
      if (/^\d+$/.test(query)) {
        this.$router.push(`/blocks/${query}`);
        return;
      }

      // 주소로 검색 (32자 이상)
      if (query.length > 30) {
        this.$router.push(`/address/${query}`);
        return;
      }

      // 해시로 검색
      this.$router.push(`/transactions/${query}`);
    },
    goToBlock(height) {
      this.$router.push(`/blocks/${height}`);
    },
    goToTransaction(hash) {
      this.$router.push(`/transactions/${hash}`);
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
    truncateHash(hash) {
      return hash.substring(0, 8) + "..." + hash.substring(hash.length - 8);
    },
    truncateAddress(address) {
      return (
        address.substring(0, 8) + "..." + address.substring(address.length - 8)
      );
    },
  },
};
</script>

<style scoped>
.home-page {
  width: 100%;
}

.hero {
  text-align: center;
  margin-bottom: 3rem;
  color: #333;
}

.hero h1 {
  font-size: 2.5rem;
  margin-bottom: 0.5rem;
  color: #667eea;
}

.hero p {
  font-size: 1.1rem;
  color: #666;
}

.search-box {
  display: flex;
  gap: 1rem;
  margin-bottom: 3rem;
}

.search-box input {
  flex: 1;
  padding: 0.75rem 1rem;
  border: 2px solid #ddd;
  border-radius: 8px;
  font-size: 1rem;
  transition: border-color 0.3s;
}

.search-box input:focus {
  outline: none;
  border-color: #667eea;
}

.search-box button {
  padding: 0.75rem 2rem;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  font-weight: bold;
  transition: transform 0.2s;
}

.search-box button:hover {
  transform: scale(1.05);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 1.5rem;
  margin-bottom: 3rem;
}

.stat-card {
  background: white;
  padding: 2rem;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
  text-align: center;
}

.stat-label {
  color: #666;
  font-size: 0.9rem;
  margin-bottom: 0.5rem;
}

.stat-value {
  font-size: 2rem;
  font-weight: bold;
  color: #667eea;
}

.recent-section {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
  gap: 2rem;
}

.section {
  background: white;
  padding: 2rem;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}

.section h2 {
  margin-bottom: 1.5rem;
  color: #333;
  border-bottom: 2px solid #667eea;
  padding-bottom: 0.5rem;
}

.blocks-list,
.transactions-list {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.list-item {
  padding: 1rem;
  border: 1px solid #eee;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.3s;
}

.list-item:hover {
  background-color: #f8f9ff;
  border-color: #667eea;
  transform: translateX(4px);
}

.item-header {
  display: flex;
  justify-content: space-between;
  margin-bottom: 0.5rem;
  font-weight: bold;
}

.block-height,
.tx-hash {
  color: #667eea;
}

.timestamp {
  color: #999;
  font-size: 0.9rem;
}

.item-detail {
  display: flex;
  justify-content: space-between;
  font-size: 0.9rem;
  color: #666;
}

.txs,
.amount {
  color: #764ba2;
}

.miner,
.status {
  color: #999;
}

.status.confirmed {
  color: #10b981;
  font-weight: bold;
}

.status.pending {
  color: #f59e0b;
  font-weight: bold;
}

.loading {
  text-align: center;
  padding: 2rem;
  color: #999;
}

@media (max-width: 768px) {
  .hero h1 {
    font-size: 1.8rem;
  }

  .search-box {
    flex-direction: column;
  }

  .search-box button {
    width: 100%;
  }

  .stats-grid {
    grid-template-columns: 1fr;
  }

  .recent-section {
    grid-template-columns: 1fr;
  }
}
</style>
