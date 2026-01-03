import { StoreProvider } from './hooks/useStore'
import { Header } from './components/Header'
import { StatusBar } from './components/StatusBar'
import { Settings } from './components/Settings'

export function App() {
  return (
    <StoreProvider>
      <Header />
      <StatusBar />
      <Settings />
    </StoreProvider>
  )
}
