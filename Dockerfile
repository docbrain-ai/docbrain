FROM node:22-alpine

# Install the DocBrain MCP server globally
RUN npm install -g docbrain-mcp@latest

# Required environment variables
ENV DOCBRAIN_API_KEY=""
ENV DOCBRAIN_SERVER_URL=""

ENTRYPOINT ["docbrain-mcp"]
