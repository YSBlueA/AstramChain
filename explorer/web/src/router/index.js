import { createRouter, createWebHistory } from 'vue-router'
import Home from '../views/Home.vue'
import Blocks from '../views/Blocks.vue'
import BlockDetail from '../views/BlockDetail.vue'
import Transactions from '../views/Transactions.vue'
import TransactionDetail from '../views/TransactionDetail.vue'
import Address from '../views/Address.vue'
import NodeStatus from '../views/NodeStatus.vue'

const routes = [
  {
    path: '/',
    name: 'Home',
    component: Home,
  },
  {
    path: '/blocks',
    name: 'Blocks',
    component: Blocks,
  },
  {
    path: '/blocks/:height',
    name: 'BlockDetail',
    component: BlockDetail,
  },
  {
    path: '/transactions',
    name: 'Transactions',
    component: Transactions,
  },
  {
    path: '/transactions/:hash',
    name: 'TransactionDetail',
    component: TransactionDetail,
  },
  {
    path: '/tx/:hash',
    name: 'TransactionDetailShort',
    component: TransactionDetail,
  },
  {
    path: '/address/:address',
    name: 'Address',
    component: Address,
  },
  {
    path: '/node/status',
    name: 'NodeStatus',
    component: NodeStatus,
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

export default router
