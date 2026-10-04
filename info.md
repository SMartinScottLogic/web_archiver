# CRAWLER
simon@proteus:~/Development/rust/web_archiver$ 
cargo run --bin web_archiver -- --workers 30 --user-agent "Garth Bot v7.1.5" --archive-dir archive --db crawler.db --min-free-space 2GiB

# Playwright Scraper (X.com)
simon@proteus:~/Development/rust/web_archiver/playwright_scraper$ 
node scraper.js ../crawler.db ../archive/json/ ../visited-pages.jsonl 

# Media first-pass processing
simon@proteus:~/Development/rust/web_archiver/archive/media/mp4$ 

trash-empty; mkdir -p ../mp4 ../png ../jpeg; rm -fv ../*.m3u8; mv -- ../*.mp4 .; mv -- ../*.png ../png/; mv -- ../*.jpg ../*.jpeg ../jpeg/; ~/Seafile/scripts/video/process ../keep/ $( ls -1Shr | tail -25 ); df -hT ~ /media/simon/*

rm -rfv ../.review-bin/

~/Development/old/fictional-succotash/fdupes -drSN ../

for i in image-*/*; do identify -format "%H %W\n" "${i}" | awk '{width=0+$1; height=0+$2; if( (width < 72 && height < 72) || ( ((width*3-height) < 5) && ((height - width*3) < 5) )) {printf "rm -vf -- \"%s\" # %s,%s\n", filename, width, height} else {printf "echo keep %s - %s,%s\n", filename,width,height}}' filename="${i}"; done | bash

xnview ..

# Video duplication + fingerprinting
simon@proteus:~/Development/rust/web_archiver/archive/media/keep$ 
RUST_BACKTRACE=1 RUST_LOG=debug cargo run --bin video_duration_sort --release -- --duration-precision 2 .; childsize . --sort Total --pattern '*' | grep -v '^1\s'; df -hT ~ /media/simon/*
