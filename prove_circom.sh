cd ./circom-verifier
docker build -t temp -f ./Dockerfile.generate-circom-verifier .
cd ../
docker create --name temp_container temp
docker cp temp_container:/tmp/aggregation/final_proof.json ./final_proof.json
docker cp temp_container:/tmp/aggregation/final_public.json ./final_public.json
docker container rm temp_container
docker rmi temp
