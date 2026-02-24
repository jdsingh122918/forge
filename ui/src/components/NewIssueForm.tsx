import { useState, useRef, useEffect } from 'react';

interface NewIssueFormProps {
  onSubmit: (title: string, description: string) => void;
  onCancel: () => void;
}

export function NewIssueForm({ onSubmit, onCancel }: NewIssueFormProps) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const titleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (title.trim()) {
      onSubmit(title.trim(), description.trim());
    }
  };

  return (
    <form
      onSubmit={handleSubmit}
      className="bg-white rounded-lg shadow-sm border border-blue-300 p-3 space-y-2"
    >
      <input
        ref={titleRef}
        type="text"
        placeholder="Issue title"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        className="w-full px-2 py-1.5 border border-gray-300 rounded text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
      />
      <textarea
        placeholder="Description (optional)"
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        rows={2}
        className="w-full px-2 py-1.5 border border-gray-300 rounded text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-none"
      />
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="px-3 py-1 text-xs text-gray-700 hover:bg-gray-100 rounded transition-colors"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={!title.trim()}
          className="px-3 py-1 text-xs font-medium text-white bg-blue-600 rounded hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          Create
        </button>
      </div>
    </form>
  );
}
