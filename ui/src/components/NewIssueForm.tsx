import { useState } from 'react';

interface NewIssueFormProps {
  onSubmit: (title: string, description: string) => void;
  onCancel: () => void;
}

export function NewIssueForm({ onSubmit, onCancel }: NewIssueFormProps) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (title.trim()) {
      onSubmit(title.trim(), description.trim());
    }
  };

  return (
    <div className="fixed inset-0 bg-black/30 flex items-center justify-center z-50" onClick={onCancel}>
      <form
        onClick={(e) => e.stopPropagation()}
        onSubmit={handleSubmit}
        className="bg-white rounded-xl shadow-xl p-6 w-full max-w-md space-y-4"
      >
        <h2 className="text-lg font-semibold text-gray-900">New Issue</h2>
        <input
          type="text"
          placeholder="Issue title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          autoFocus
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
        />
        <textarea
          placeholder="Description (optional)"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          rows={4}
          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent resize-none"
        />
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2 text-sm text-gray-700 hover:bg-gray-100 rounded-md transition-colors"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={!title.trim()}
            className="px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          >
            Create
          </button>
        </div>
      </form>
    </div>
  );
}
