# TOCTOU Race in create_issue

This code is from the factory database layer that manages a kanban board.
`create_issue` inserts a new issue into the board at the next available position
within a column. It first queries the maximum position, then inserts at max + 1.

The MAX(position) query runs on the bare connection before the transaction begins.
Under concurrent access, two calls can read the same MAX(position) value and both
insert at the same position, producing duplicate positions in the column.

The fix is to move the MAX(position) query inside the transaction so that the
read and write are atomic.
