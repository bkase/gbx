#!/usr/bin/env bash
# Validates commit message format:
# - Subject line: max 50 characters, imperative present-tense
# - Blank line between subject and body
# - Body lines: max 72 characters

set -euo pipefail

commit_msg=$(cat "$1")
first_line=$(echo "$commit_msg" | head -n1)
subject_length=${#first_line}

# Check subject line length (max 50 characters)
if [ "$subject_length" -gt 50 ]; then
  echo "Error: Commit subject line is too long ($subject_length characters, max 50)"
  echo "Keep the first line concise and add details in the body after a blank line."
  echo ""
  echo "Your subject: $first_line"
  exit 1
fi

# Check for past tense (should be imperative present-tense)
if echo "$first_line" | grep -qE '^(Added|Fixed|Updated|Removed|Changed|Deleted|Created|Refactored)'; then
  echo "Error: Commit message should use imperative present-tense:"
  echo "  Use 'Add' not 'Added'"
  echo "  Use 'Fix' not 'Fixed'"
  echo "  Use 'Update' not 'Updated'"
  echo ""
  echo "Your subject: $first_line"
  exit 1
fi

# Ensure it starts with a capital letter and a verb
if ! echo "$first_line" | grep -qE '^[A-Z][a-z]+ '; then
  echo "Error: Commit subject should start with an imperative verb (e.g., 'Add', 'Fix', 'Update')"
  echo "Your subject: $first_line"
  exit 1
fi

# Check for blank line between subject and body if body exists
line_count=$(echo "$commit_msg" | wc -l | tr -d ' ')
if [ "$line_count" -gt 1 ]; then
  second_line=$(echo "$commit_msg" | sed -n '2p')
  if [ -n "$second_line" ]; then
    echo "Error: Commit message must have a blank line between subject and body"
    echo ""
    echo "Format:"
    echo "  Subject line (max 50 chars)"
    echo "  "
    echo "  Detailed explanation in the body (wrap at 72 chars)..."
    exit 1
  fi
fi

# Check body line lengths (max 72 characters), skipping subject and blank line
if [ "$line_count" -gt 2 ]; then
  body_lines=$(echo "$commit_msg" | tail -n +3)
  line_num=3
  while IFS= read -r line; do
    line_length=${#line}
    if [ "$line_length" -gt 72 ]; then
      echo "Error: Body line $line_num is too long ($line_length characters, max 72)"
      echo "Wrap body text at 72 characters for readability."
      echo ""
      echo "Line $line_num: $line"
      exit 1
    fi
    line_num=$((line_num + 1))
  done <<< "$body_lines"
fi
