import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import StatusMessage from './components/StatusMessage';
import './styles/globals.css';

function App() {
  const [statusMessage, setStatusMessage] = useState('Welcome to Krillnotes');

  useEffect(() => {
    // Listen for menu events from Rust backend
    const unlisten = listen<string>('menu-action', (event) => {
      setStatusMessage(event.payload);
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center">
      <div className="text-center">
        <h1 className="text-4xl font-bold mb-4">Krillnotes</h1>
        <StatusMessage message={statusMessage} />
      </div>
    </div>
  );
}

export default App;
