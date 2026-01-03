import { StoreProvider } from './hooks/useStore'
import { Header } from './components/Header'
import { Settings } from './components/Settings'

export function App() {
  return (
    <StoreProvider>
      <Header />
      <Settings />
    </StoreProvider>
  )
}
