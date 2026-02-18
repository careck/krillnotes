interface StatusMessageProps {
  message: string;
  isError?: boolean;
}

function StatusMessage({ message, isError = false }: StatusMessageProps) {
  return (
    <div className={`mt-4 p-4 rounded-lg ${
      isError
        ? 'bg-red-500/10 border border-red-500/20 text-red-500'
        : 'bg-secondary'
    }`}>
      <p className="text-sm">{message}</p>
    </div>
  );
}

export default StatusMessage;
