interface StatusMessageProps {
  message: string;
}

function StatusMessage({ message }: StatusMessageProps) {
  return (
    <div className="p-4 rounded-lg bg-secondary">
      <p className="text-sm text-secondary-foreground">{message}</p>
    </div>
  );
}

export default StatusMessage;
