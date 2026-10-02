FROM mcr.microsoft.com/playwright:v1.59.1-noble

WORKDIR /app/playwright_scraper

ENV CDP_ENDPOINT=http://127.0.0.1:9222

COPY package.json package-lock.json ./
RUN npm ci --omit=dev

COPY scraper.js ./
RUN mkdir -p /data/archive /data/chrome-profile \
    && ln -s /data/crawler.db /app/crawler.db \
    && ln -s /data/archive /app/archive \
    && ln -s /data/visited-pages.jsonl /app/visited-pages.jsonl \
    && chown -R 10001:10001 /app /data

USER 10001:10001
CMD ["node", "scraper.js", "../crawler.db", "../archive/json/", "../visited-pages.jsonl"]
