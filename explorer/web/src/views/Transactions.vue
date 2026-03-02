<template>
  <div class="transactions-page">
    <h1>Transaction List</h1>

    <div v-if="transactions.length" class="transactions-table">
      <table>
        <thead>
          <tr>
            <th>Type</th>
            <th>Hash</th>
            <th>From</th>
            <th>To</th>
            <th>Amount</th>
            <th>Fee</th>
            <th>Status</th>
            <th>Time</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="tx in transactions"
            :key="tx.hash"
            @click="goToTransactionDetail(tx.hash)"
            class="table-row"
          >
            <td class="tx-type">
              <span v-if="tx.from === 'Block_Reward'" class="tx-badge coinbase" title="Mining Reward">⛏️</span>
              <span v-else class="tx-badge transfer" title="Transfer">💸</span>
            </td>
            <td class="hash">{{ truncateHash(tx.hash) }}</td>
            <td class="address">{{ truncateAddress(tx.from) }}</td>
            <td class="address">{{ truncateAddress(tx.to) }}</td>
            <td class="amount">{{ formatAmount(tx.amount) }}</td>
            <td class="fee">{{ formatAmount(tx.fee) }}</td>
            <td class="status" :class="tx.status">{{ tx.status }}</td>
            <td class="timestamp">{{ formatTime(tx.timestamp) }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-else class="loading">Loading...</div>

    <div class="pagination">
      <button
        @click="previousPage"
        :disabled="currentPage === 1"
        class="nav-btn"
      >
        Previous
      </button>
      <span class="page-info">{{ currentPage }} / {{ totalPages }}</span>
      <button
        @click="nextPage"
        :disabled="currentPage >= totalPages"
        class="nav-btn"
      >
        Next
      </button>
    </div>
  </div>
</template>

<script>
import { explorerAPI } from "../api/explorer";

export default {
  name: "Transactions",
  data() {
    return {
      transactions: [],
      currentPage: 1,
      limit: 20,
      total: 0,
    };
  },
  computed: {
    totalPages() {
      return Math.ceil(this.total / this.limit);
    },
  },
  mounted() {
    this.fetchTransactions();
  },
  methods: {
    async fetchTransactions() {
      try {
        const res = await explorerAPI.getTransactions(
          this.currentPage,
          this.limit
        );
        this.transactions = res.data.transactions;
        this.total = res.data.total;
      } catch (error) {
        console.error("Failed to load transactions:", error);
      }
    },
    previousPage() {
      if (this.currentPage > 1) {
        this.currentPage--;
        this.fetchTransactions();
      }
    },
    nextPage() {
      if (this.currentPage < this.totalPages) {
        this.currentPage++;
        this.fetchTransactions();
      }
    },
    goToTransactionDetail(hash) {
      this.$router.push(`/transactions/${hash}`);
    },
    formatTime(timestamp) {
      const date = new Date(timestamp);
      return date.toLocaleString("ko-KR");
    },
    formatAmount(value) {
      // Handle hex string (0x...), decimal string, number, and U256 array format
      let num;
      
      if (Array.isArray(value)) {
        num = BigInt(value[0]) + (BigInt(value[1]) << BigInt(64)) +
              (BigInt(value[2]) << BigInt(128)) + (BigInt(value[3]) << BigInt(192));
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
      const ASRM = Number(num) / Number(divisor);

      return ASRM.toLocaleString("en-US", {
        minimumFractionDigits: 0,
        maximumFractionDigits: 6,
      });
    },
    truncateHash(hash) {
      return hash.substring(0, 12) + "...";
    },
    truncateAddress(address) {
      return (
        address.substring(0, 8) + "..." + address.substring(address.length - 4)
      );
    },
  },
};
</script>

<style scoped>
.transactions-page {
  background: white;
  padding: 2rem;
  border-radius: 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
}

h1 {
  margin-bottom: 2rem;
  color: #667eea;
}

.transactions-table {
  overflow-x: auto;
  margin-bottom: 2rem;
}

table {
  width: 100%;
  border-collapse: collapse;
}

thead {
  background-color: #f5f5f5;
}

th {
  padding: 1rem;
  text-align: left;
  font-weight: bold;
  color: #333;
  border-bottom: 2px solid #ddd;
}

td {
  padding: 0.75rem 1rem;
  border-bottom: 1px solid #eee;
}

.table-row {
  cursor: pointer;
  transition: background-color 0.3s;
}

.table-row:hover {
  background-color: #f8f9ff;
}

.tx-type {
  text-align: center;
  width: 50px;
}

.tx-badge {
  font-size: 1.2rem;
  cursor: help;
}

.tx-badge.coinbase {
  filter: drop-shadow(0 0 2px #f59e0b);
}

.tx-badge.transfer {
  filter: drop-shadow(0 0 2px #667eea);
}

.hash {
  color: #667eea;
  font-family: "Courier New", monospace;
  font-size: 0.85rem;
}

.address {
  font-family: "Courier New", monospace;
  font-size: 0.85rem;
}

.amount {
  color: #10b981;
  font-weight: bold;
}

.fee {
  color: #f59e0b;
}

.status {
  font-weight: bold;
  padding: 0.25rem 0.5rem;
  border-radius: 4px;
}

.status.confirmed {
  color: #10b981;
  background-color: #d1fae5;
}

.status.pending {
  color: #f59e0b;
  background-color: #fef3c7;
}

.pagination {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 1rem;
}

.nav-btn {
  padding: 0.5rem 1rem;
  background-color: #667eea;
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  transition: opacity 0.3s;
}

.nav-btn:hover:not(:disabled) {
  opacity: 0.8;
}

.nav-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.page-info {
  font-weight: bold;
  color: #666;
}

.loading {
  text-align: center;
  padding: 3rem;
  color: #999;
}
</style>

