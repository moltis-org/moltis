.PHONY: run clean

run:
	nix develop -c cargo run --manifest-path Cargo.toml -- -i ../data-locate-pipelight-shell-feb-28.txt --project-name pipelight-jobs --worker-name main-worker

clean:
	rm -rf target output