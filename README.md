## Installation
### Docker
```shell
docker build -t "file-reader:latest" ./
docker run -it --rm -v /mylogs:/logs -p 80:8000 file-reader:latest

docker run --rm -v file-reader:latest /file_reader --help
```
